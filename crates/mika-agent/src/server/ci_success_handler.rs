//! Structural handler for `check_suite.completed(success)` webhook events.
//!
//! Intercepts CI success events **before** the LLM turn and re-evaluates
//! merge eligibility for PRs that have a pending `VERDICT: pass` but were
//! blocked on CI at the time of approval. This removes the merge recovery
//! from LLM improvisation — it's a state-machine transition, not a judgement call.
//!
//! Companion to `verdict_handler` (which handles `pull_request_review.submitted`).
//! See issue #571.
//!
//! ## Invariants
//!
//! - `check_suite.completed/success` is scoped to ONE workflow. A PR with
//!   multiple workflows can receive this event while another workflow is still
//!   running or already failed. We MUST aggregate across all required checks
//!   here — the webhook alone is not sufficient evidence that all CI is green.
//!   This is the actual gate, not a defensive re-check.
//!
//! - Structural handlers MUST return Passthrough for non-matching event types.
//!   Call order is irrelevant — handlers are disjoint on event_type.
//!
//! - Loop prevention: post-merge, GitHub fires `check_suite.completed/success`
//!   with `head_branch = main` (or whatever the base is). `gh pr list --head main
//!   --state open` returns empty → NoPr → no-op. Self-terminating by construction.
//!
//! - Stale verdict policy: if QA approved a different SHA than HEAD, we do NOT merge.
//!   A push after approval — even a mechanical fix — is unreviewed code.
//!   The cost is one extra QA cycle. The alternative is silently trusting
//!   that the push was safe, which is the judgment call we're trying to avoid here.
//!
//! - Porte d'entrée (mika#2260) : ce handler **n'évalue que dans le dispatcher**.
//!   `try_handle_ci_success` refuse en tête — après la sélection par type
//!   d'événement, avant l'exigence de token, avant tout appel `gh`, avant toute
//!   écriture de dedup — quand `owns_merge_transition(db.agent_id())` est faux, et
//!   rend alors `Passthrough { enrichment: None }` plus une ligne d'audit
//!   `ci_success_handler_skipped_not_merge_actor`. #2248 avait mis l'identité à la
//!   **sortie** (qui merge) ; elle était absente de l'**entrée** (pour qui cette
//!   évaluation existe), si bien que le relecteur parcourait tout l'évaluateur
//!   avant d'être retenu à l'acteur. L'ordre de la porte est épinglé par
//!   `tests/eval/test_ci_success_handler.rs`, l'attribution par
//!   `tests/eval/test_ci_success_fanout_2260.rs`.
//!
//! - Signal, pas acteur (mika#2248): ce handler **ne merge pas**. Il évalue, et
//!   émet un signal merge-ready ([`MergeReadySignal`]) que le dispatcher
//!   (`mika-dev`) consomme dans [`super::merge_ready_handler`] pour merger sous
//!   sa propre identité. La raison est structurelle : `check_suite.completed
//!   (success)` est diffusé à `mika-dev` ET `mika-qa` (fan-out mika#1711), les
//!   deux exécutent ce handler, et un `gh pr merge` posé ici tourne sous le token
//!   de celui qui gagne la course. Mesuré le 2026-09-08 sur mika#2244 :
//!   `mergedBy = mika-platform-qa`, le relecteur mergeant sa propre approbation.
//!   Corollaire : aucun `run_gh_merge` dans ce fichier — épinglé par
//!   `tests/eval/test_ci_success_handler.rs`.
//!
//! - Burst dedup (mika#1869): a single push fans out to up to 8 workflows, each
//!   firing its own `check_suite.completed(success)` webhook, and every one walks
//!   this full path doing identical work (the aggregation above re-runs on each).
//!   Two `head_sha`-keyed layers collapse the burst to one evaluation, placed
//!   right after `find_open_pr` resolves `head_sha`: (1) an in-memory precise gate
//!   ([`super::check_suite_dedup`], `ci_success_dedup.skip` log) and (2) an
//!   audit-durable gate (`ci_success_handler_processed` marker rows +
//!   `count_recent_audit_events_for_target`, `ci_success_handler.dedup_skip` log)
//!   that survives a process restart the in-memory map cannot. Both key on
//!   `(repo, branch, head_sha)` / `pr:{repo}#{n}@{head_sha}`, so genuine distinct
//!   pushes (new `head_sha`) are never conflated. The audit read fails open.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::perimeter::{self, Classification};
use crate::task_state::merge_metadata;
use crate::tools::pr_merge_with_gate::{
    BehindMainInfo, BehindMainRemediation, CheckClassification, classify_checks,
    describe_behind_main_remediation, is_behind_main, remediate_behind_main, run_gh_checks,
    run_gh_pr_view, run_gh_subprocess,
};
use mika_common::forge_identity::{DISPATCHER_AGENT, MergeReadySignal, owns_merge_transition};

use super::check_suite_dedup;
use super::verdict::{Verdict, parse_verdict};
use super::verdict_handler::VerdictAction;

/// Parsed fields from a gateway-formatted check_suite success event.
pub(crate) struct CheckSuiteEvent {
    pub(crate) repo: String,
    pub(crate) branch: String,
}

/// Parse the gateway-formatted check_suite success event text.
///
/// Expected format: `[GitHub] Check suite success on {repo} (branch: {branch})`
pub(crate) fn parse_check_suite_success(text: &str) -> Option<CheckSuiteEvent> {
    let first_line = text.lines().next()?;

    // Must be a check_suite success event
    if !first_line.starts_with("[GitHub] Check suite success on ") {
        return None;
    }

    // Extract repo: between "on " and " (branch:"
    let after_on = first_line.strip_prefix("[GitHub] Check suite success on ")?;
    let branch_marker = " (branch: ";
    let marker_pos = after_on.find(branch_marker)?;
    let repo = &after_on[..marker_pos];

    // Extract branch: between "(branch: " and closing ")"
    let after_marker = &after_on[marker_pos + branch_marker.len()..];
    let branch = after_marker.strip_suffix(')')?;

    Some(CheckSuiteEvent {
        repo: repo.to_string(),
        branch: branch.to_string(),
    })
}

/// Decide whether a prior `ci_success_handler_processed` audit marker means this
/// event is a cross-restart duplicate (AC2, mika#1869). Pure so it can be unit
/// tested without touching the network or the `gh` subprocess entry point.
pub(crate) fn is_duplicate_processed(recent_marker_count: i64) -> bool {
    recent_marker_count >= 1
}

/// Clé d'une tête de PR dans l'audit : `pr:{repo}#{n}@{head_sha}`. Partagée
/// par le marqueur de dédup (mika#1869) et l'attestation DECISION-CORE
/// (mika#2675) — une seule forme, que le pré-filtre relit telle quelle.
pub(crate) fn head_key(repo: &str, pr_number: u64, head_sha: &str) -> String {
    format!("pr:{repo}#{pr_number}@{head_sha}")
}

/// `audit_events.tool_name` de l'attestation « cette tête a été décidée
/// DECISION-CORE », écrite APRÈS l'envoi de la notification (mika#2675
/// phase 3). Cible [`head_key`] ; `after_value` vaut
/// [`DECISION_CORE_HOLD_NOTIFIED`] ou [`DECISION_CORE_HOLD_NOT_NOTIFIED`].
/// **SOLE WRITER** : [`try_handle_ci_success`]. Lecteur : le pré-filtre.
pub(crate) const DECISION_CORE_HOLD_TOOL: &str = "ci_success_handler_decision_core_hold";
pub(crate) const DECISION_CORE_HOLD_NOTIFIED: &str = "notified";
pub(crate) const DECISION_CORE_HOLD_NOT_NOTIFIED: &str = "not_notified";

/// L'issue de l'envoi, telle que l'attestation la porte. Seul `Delivered`
/// atteste une notification : un canal absent ou un échec n'en est pas une.
pub(crate) fn decision_core_notification_value(
    outcome: &anyhow::Result<crate::messaging::SendOutcome>,
) -> (&'static str, String) {
    use crate::messaging::SendOutcome;
    match outcome {
        Ok(SendOutcome::Delivered) => {
            (DECISION_CORE_HOLD_NOTIFIED, "notification=delivered".into())
        }
        Ok(SendOutcome::Failed { reason }) => (
            DECISION_CORE_HOLD_NOT_NOTIFIED,
            format!("notification=failed reason={reason}"),
        ),
        Ok(SendOutcome::NoChannel) => (
            DECISION_CORE_HOLD_NOT_NOTIFIED,
            "notification=no_channel".into(),
        ),
        Err(e) => (
            DECISION_CORE_HOLD_NOT_NOTIFIED,
            format!("notification=error error={e}"),
        ),
    }
}

/// Attempt to handle a CI success event structurally before the LLM turn.
///
/// Returns `VerdictAction::Handled` when the handler emitted a merge-ready
/// signal (or encountered an evaluation error), with a pre-digest message for
/// the LLM. It never merges — see the `mika#2248` invariant above.
/// Returns `VerdictAction::Passthrough` for all other cases (non-matching events,
/// no open PR, no QA verdict, stale verdict, checks not all green).
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ci_success(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Parse the check_suite success event from formatted text
    let event = match parse_check_suite_success(text) {
        Some(e) => e,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    // 1b. Porte d'entrée — seul le dispatcher évalue (mika#2260).
    //
    // La transition évaluer→signaler→merger est un tout, et elle appartient au
    // dispatcher. Un agent qui ne peut pas consommer le signal n'a aucune raison
    // de l'émettre : son évaluation n'a, par construction, aucun consommateur —
    // le signal voyage en bande dans `req.text`, relu par le handler suivant *du
    // même agent*, où l'acteur le retient.
    //
    // Le placement est le point porteur, et il est dicté par les effets mesurés le
    // 2026-09-09/10, dans l'ordre d'exécution : la clé de dedup de l'étape 2b est
    // **globale au processus** et clé sur `{repo}:{branch}:{head_sha}` sans
    // `agent_id`, donc le relecteur entrant en premier consommait le créneau du
    // dispatcher (mesuré à 5 ms près le 09:12:36 — un deferral à 59,9 s au lieu
    // de 60,005 s laissait la PR verte et ouverte sans que rien ne la ferme) ;
    // s'y ajoutaient cinq appels `gh` sous le PAT du relecteur, une écriture
    // `PUT …/update-branch` à l'étape 5c, et une notification opérateur
    // DECISION-CORE en double. La porte précède donc **tout** : le `evaluating`
    // ci-dessous, l'exigence de token, le dedup, et toute ligne d'audit.
    //
    // `Passthrough { enrichment: None }` et non `Handled` : le tour du relecteur
    // doit voir l'**événement brut**, que `qa-review-webhook-success` attend pour
    // corréler la PR — le pré-digest de l'évaluateur remplaçait `req.text`. Et
    // sans enrichissement : le prompt du relecteur sait déjà qu'il ne merge pas,
    // lui redire n'ajoute qu'un second texte à suivre.
    //
    // La seconde ceinture de mika#2248 (`authorize_merge`, le refus outil) n'est
    // pas remplacée : après cette porte le hold du relecteur devient inatteignable
    // en production, mais il reste la défense si un signal arrivait par un chemin
    // que cet évaluateur n'a pas posé.
    let agent_id = db.agent_id();
    if !owns_merge_transition(agent_id) {
        info!(
            event = "ci_success_handler_skipped_not_merge_actor",
            repo = %event.repo,
            branch = %event.branch,
            agent_id = %agent_id,
            dispatcher = DISPATCHER_AGENT,
            "CI success: cet agent ne possède pas la transition de merge — aucune évaluation, \
             l'événement brut passe à son tour (mika#2260)"
        );

        // La clé cible porte `{repo}@{branch}` et non un numéro de PR : celui-ci
        // n'est pas connu ici, et ne doit pas l'être — le connaître coûterait le
        // premier appel `gh`, c'est-à-dire l'effet de bord que cette porte retire.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ci_success_handler_skipped_not_merge_actor",
                &format!("event:{}@{}", event.repo, event.branch),
                None,
                None,
                Some(&format!(
                    "trigger=check_suite_success agent_id={agent_id} dispatcher={DISPATCHER_AGENT}"
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(
                error = %e,
                agent_id = %agent_id,
                "Failed to log ci_success_handler_skipped_not_merge_actor audit event (continuing)"
            );
        }

        return VerdictAction::Passthrough { enrichment: None };
    }

    info!(
        repo = %event.repo,
        branch = %event.branch,
        "CI success handler: evaluating merge eligibility"
    );

    // Require GitHub token for merge operations
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                repo = %event.repo,
                branch = %event.branch,
                "CI success but no GitHub token available — cannot evaluate merge"
            );
            return VerdictAction::Passthrough {
                enrichment: Some(
                    "[ci_success_handler] CI checks passed but no GitHub token configured. \
                     Cannot evaluate merge eligibility.\n\n"
                        .to_string(),
                ),
            };
        }
    };

    // 2. Find open PR for this branch
    let pr = match find_open_pr(&event.repo, &event.branch, token).await {
        Ok(Some(pr)) => pr,
        Ok(None) => {
            info!(
                repo = %event.repo,
                branch = %event.branch,
                "CI success but no open PR for branch — likely post-merge webhook"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                repo = %event.repo,
                branch = %event.branch,
                "Failed to look up PR for branch"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 2b. Dedup layer 1 — in-memory, precise (AC1, mika#1869).
    //
    // A single push triggers up to 8 workflows; each fires its own
    // check_suite.completed(success) webhook, and every one walks this full path
    // (gh calls + merge attempt) doing identical work. head_sha is only known now
    // (find_open_pr resolved it), so this is the earliest precise dedup point.
    // Distinct pushes advance head_sha and are never conflated.
    if check_suite_dedup::try_dedup_check_suite(
        &event.repo,
        &event.branch,
        &pr.head_sha,
        check_suite_dedup::DEDUP_WINDOW,
    ) {
        info!(
            event = "ci_success_dedup.skip",
            repo = %event.repo,
            branch = %event.branch,
            head_sha = %pr.head_sha,
            pr_number = pr.number,
            "CI success dedup: duplicate check_suite event within window — skipping merge evaluation"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 2c. Dedup layer 2 — audit-durable, cross-restart (AC2, mika#1869).
    //
    // The in-memory map above is empty after a process restart; the audit trail
    // persists. A re-fired event within the window still dedups via a marker row.
    // Scope note: count_recent_audit_events_for_target counts within this agent's
    // agent_id scope, and since mika#2260 only the dispatcher reaches this point —
    // the entry gate turns every other agent away above. Before that gate, this
    // note claimed check_suite events "route to the same agent (mika-qa)", which
    // has been false since the mika#1711 fan-out: they reach mika-dev AND mika-qa,
    // both ran this path, and the in-memory dedup key one hop above is process-global
    // and carries no agent_id — so the reviewer's entry consumed the dispatcher's
    // slot. Fail-open on read error — never let an audit hiccup block a legitimate
    // merge.
    let processed_key = head_key(&event.repo, pr.number, &pr.head_sha);
    let since = crate::timestamp::now_minus(chrono::Duration::seconds(60));
    match db
        .count_recent_audit_events_for_target(
            "ci_success_handler_processed",
            &processed_key,
            &since,
        )
        .await
    {
        Ok(count) if is_duplicate_processed(count) => {
            info!(
                event = "ci_success_handler.dedup_skip",
                repo = %event.repo,
                pr_number = pr.number,
                head_sha = %pr.head_sha,
                prior_markers = count,
                "CI success dedup: prior processing marker found within window — skipping merge evaluation"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Ok(_) => {
            // First real processing for this (repo, pr, head_sha) — write the
            // marker early (before find_pass_verdict) so concurrent siblings that
            // cleared the empty DashMap on a cold start still find the row.
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "ci_success_handler_processed",
                    &processed_key,
                    None,
                    None,
                    Some("trigger=check_suite_success dedup_marker"),
                    Some(trace_id),
                )
                .await
            {
                warn!(
                    error = %e,
                    pr_number = pr.number,
                    "Failed to write ci_success_handler_processed dedup marker (continuing)"
                );
            }
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                "CI success dedup: audit-count query failed — proceeding (fail-open)"
            );
        }
    }

    // 3. Find QA pass verdict
    let verdict_review = match find_pass_verdict(pr.number, &event.repo, token).await {
        Ok(Some(review)) => review,
        Ok(None) => {
            info!(
                pr_number = pr.number,
                repo = %event.repo,
                "CI success but no VERDICT: pass review found on PR"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                repo = %event.repo,
                "Failed to fetch PR reviews"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 4. Stale-SHA gate: review.commit_id must match pr.head.sha
    if verdict_review.commit_id != pr.head_sha {
        info!(
            pr_number = pr.number,
            repo = %event.repo,
            review_sha = %verdict_review.commit_id,
            head_sha = %pr.head_sha,
            reviewer = %verdict_review.reviewer,
            "CI success but VERDICT: pass was for a different SHA — stale verdict"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 5. CI aggregation gate (load-bearing — check_suite event is per-workflow)
    let checks_future = run_gh_checks(pr.number, &event.repo, token);
    let checks = match tokio::time::timeout(std::time::Duration::from_secs(60), checks_future).await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                "Failed to fetch CI checks for CI success merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(&event, pr.number, &e),
            };
        }
        Err(_) => {
            warn!(
                pr_number = pr.number,
                "CI check fetch timed out after 60s for CI success merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(
                    &event,
                    pr.number,
                    "CI check fetch timed out after 60s",
                ),
            };
        }
    };

    let classification = classify_checks(&checks);

    if classification != CheckClassification::AllPassed {
        // Un seul `match` rend les deux moitiés de ce que la ligne dira. Deux
        // discriminations successives sur la même valeur — une pour le détail,
        // une pour décider de la relance — étaient deux endroits libres de
        // diverger sur « ce bras est-il le rouge ? ».
        //
        // La relance (mika#2617 U3/AC2) est un effet de bord, jamais une
        // décision : ce chemin rend `Passthrough` quoi qu'il arrive.
        //
        // **Aucun appel réseau ajouté** : `find_open_pr` a déjà résolu
        // `pr.head_sha` à l'étape 2b — c'est la ligne la plus favorable du
        // tableau de R11, et c'est elle qui rend la relance gratuite ici.
        //
        // L'issue vit sur la ligne de journal, pas dans un `enrichment` : ce
        // handler rend délibérément `enrichment: None` (il évalue, il ne parle
        // pas au modèle — mika#2260), et U3 n'a pas pour périmètre de changer
        // ce contrat.
        let (detail, rerun_note) = match classification {
            CheckClassification::HasFailures => {
                let failing: Vec<String> = checks
                    .iter()
                    .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                    .map(|c| format!("  - {} ({})", c.name, c.state))
                    .collect();
                let rerun = crate::merge_gate_rerun::maybe_rerun_failed_checks(
                    db,
                    session_id,
                    trace_id,
                    &event.repo,
                    pr.number,
                    &pr.head_sha,
                    &checks,
                    token,
                )
                .await;
                (
                    format!("failing checks:\n{}", failing.join("\n")),
                    crate::merge_gate_rerun::rerun_detail_suffix(&rerun),
                )
            }
            CheckClassification::HasPending => {
                let pending: Vec<String> = checks
                    .iter()
                    .filter(|c| c.bucket == "pending")
                    .map(|c| format!("  - {} ({})", c.name, c.state))
                    .collect();
                (
                    format!("pending checks:\n{}", pending.join("\n")),
                    String::new(),
                )
            }
            CheckClassification::AllPassed => unreachable!(),
        };

        info!(
            pr_number = pr.number,
            repo = %event.repo,
            "CI success event for one workflow but not all checks pass yet: {detail}{rerun_note}"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 5b. Forge-gate perimeter check (mika#1853 — coupled pair with verdict_handler mika#1829).
    //
    // The CI-success race path was previously the load-bearing bypass: `verdict_handler`
    // has consulted the classifier since mika#1829, but this handler called
    // `run_gh_merge` directly — mika#1851 auto-merged 4 DECISION-CORE files
    // (crates/mika-agent/src/task_engine/**, server/mod.rs) through this callsite on
    // 2026-07-27T08:09:14Z because the perimeter classifier was never consulted.
    //
    // Fail-closed: on gh-CLI fetch error, we treat the PR as DECISION-CORE (better to
    // hold a MECHANICAL PR than to auto-merge a DECISION-CORE PR through the CI-success
    // race path). Behavioral, not declarative — the diff is authority; labels and PR
    // body prose are never consulted.
    let perimeter_verdict = match perimeter::fetch::fetch_pr_files(pr.number, &event.repo, token)
        .await
    {
        Ok(files) => perimeter::classify_pr_files(&files),
        Err(e) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                repo = %event.repo,
                "Perimeter classifier: failed to fetch PR files — fail-closed to DECISION-CORE (mika#1853)"
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
            event = "ci_success_handler_human_gate_required",
            pr_number = pr.number,
            repo = %event.repo,
            summary = %perimeter_verdict.summary(),
            decision_core_files = ?perimeter_verdict.decision_core_files,
            "Forge-gate: PR touches DECISION-CORE zone(s) — skipping CI-success auto-merge, operator must merge manually (mika#1853)"
        );

        // Audit event so operators can grep for gate firings independent of the merge path.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ci_success_handler_human_gate_required",
                &format!("pr:{}#{}", event.repo, pr.number),
                None,
                None,
                Some(&format!(
                    "trigger=check_suite_success gate=forge-gate reviewer={} decision_core_files={}",
                    verdict_review.reviewer,
                    perimeter_verdict.decision_core_files.join(","),
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log ci_success_handler_human_gate_required audit event");
        }

        // Notify operator with the concrete file list.
        let (notified_value, notified_detail) = if let Some(sender) = message_sender {
            let notification = format!(
                "PR #{} on {} — CI checks all green + VERDICT: pass from @{}, but touches DECISION-CORE zone(s). \
                 Auto-merge blocked by forge-gate (mika#1853). {}. Operator must merge manually.",
                pr.number,
                event.repo,
                verdict_review.reviewer,
                perimeter_verdict.summary(),
            );
            let outcome = sender.send(&notification).await;
            match &outcome {
                Ok(crate::messaging::SendOutcome::Delivered) => {}
                Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                    warn!(reason = %reason, "CI success DECISION-CORE hold notification delivery failed");
                }
                Ok(crate::messaging::SendOutcome::NoChannel) => {
                    warn!(
                        "CI success DECISION-CORE hold notification skipped — no reply channel (chat_id=0)"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to send CI success DECISION-CORE hold notification");
                }
            }
            decision_core_notification_value(&outcome)
        } else {
            (
                DECISION_CORE_HOLD_NOT_NOTIFIED,
                "notification=no_sender".to_string(),
            )
        };

        // mika#2675 phase 3 — attestation par TÊTE, écrite APRÈS l'envoi pour
        // porter son issue. La ligne `human_gate_required` ci-dessus cible
        // `pr:{repo}#{n}` sans SHA et précède l'envoi : elle ne peut attester ni
        // la tête ni la notification. Celle-ci le peut, et c'est ce que le
        // pré-filtre relit pour écarter le tour LLM d'un retraitement qui
        // redécide la même chose sur la même tête. Additive : aucune décision,
        // aucun envoi, aucun retour de ce handler n'en dépend.
        if let Err(e) = db
            .log_audit_event(
                session_id,
                DECISION_CORE_HOLD_TOOL,
                &head_key(&event.repo, pr.number, &pr.head_sha),
                None,
                Some(notified_value),
                Some(&notified_detail),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log ci_success_handler_decision_core_hold audit event");
        }

        return VerdictAction::Handled {
            pre_digest: format_decision_core_hold_pre_digest(
                &event,
                pr.number,
                &verdict_review.reviewer,
                &perimeter_verdict.summary(),
            ),
        };
    }

    // 5c. Behind-main assertion (#1577) + remediation (mika#2238).
    // Fetch the PR's baseRefOid via gh pr view, then compare against main HEAD.
    // When behind, GitHub is asked to update the branch and this turn ENDS.
    //
    // Placed AFTER the perimeter gate on purpose: this step WRITES (it moves the
    // PR's head), and a DECISION-CORE PR must be handed to the operator before
    // the loop touches a branch they own. It stays after the all-checks-green
    // gate above for the same reason it does in `pr_merge_with_gate` — a red PR
    // is reported red, not "awaiting fresh CI".
    //
    // Fail-open on the DETECTION API error, as before.
    match run_gh_pr_view(pr.number, &event.repo, token).await {
        Ok(preflight) => {
            match is_behind_main(&preflight.base_ref_oid, &event.repo, token).await {
                Ok(Some(info)) => {
                    let remediation = remediate_behind_main(
                        "ci_success_handler",
                        pr.number,
                        &event.repo,
                        &preflight.base_ref_name,
                        token,
                        &info,
                    )
                    .await;
                    if let Some(enrichment) = format_behind_main_enrichment(&remediation, &info) {
                        return VerdictAction::Passthrough {
                            enrichment: Some(enrichment),
                        };
                    }
                    // `None` — the PR turned out not to be behind. Proceed.
                }
                Ok(None) => {} // Up-to-date — proceed to merge
                Err(e) => {
                    warn!(
                        pr_number = pr.number,
                        error = %e,
                        "CI success handler: failed to check behind-main — proceeding (fail-open)"
                    );
                }
            }
        }
        Err(e) => {
            warn!(
                pr_number = pr.number,
                error = %e.message,
                "CI success handler: failed to fetch PR preflight for behind-main check — proceeding (fail-open)"
            );
        }
    }

    // 6. Toutes les portes sont franchies — ÉMETTRE LE SIGNAL, ne pas merger (mika#2248).
    //
    // Ce handler tourne dans `mika-dev` ET dans `mika-qa` (fan-out mika#1711) ;
    // un `gh pr merge` ici merge sous le token de celui qui gagne la course. Le
    // signal est inerte : il dit « cette PR a franchi toutes les portes pour ce
    // head_sha », et laisse le dispatcher agir sous sa propre identité.
    //
    // L'identité décide à l'**entrée** si cette évaluation a un consommateur
    // (mika#2260, la porte 1b ci-dessus) ; elle ne décide jamais **ici** qui
    // merge — cela reste l'affaire de l'acteur, et cette ligne-là n'a pas bougé.
    let pr_url = format!("https://github.com/{}/pull/{}", event.repo, pr.number);
    let task = match db.find_active_task_by_pr_url(&pr_url).await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, pr_url = %pr_url, "Failed to look up task by PR URL");
            None
        }
    };
    let task_id = task
        .as_ref()
        .map(|t| t.id.as_str())
        .unwrap_or("none")
        .to_string();

    let signal = MergeReadySignal {
        repo: event.repo.clone(),
        pr_number: pr.number,
        head_sha: pr.head_sha.clone(),
        branch: event.branch.clone(),
        task_id: task_id.clone(),
        reviewer_login: verdict_review.reviewer.clone(),
    };

    // Trace durable côté tâche : l'état de sortie de CE handler est « signalé »,
    // jamais « mergé ». L'acteur écrit le sien (`merge_initiated`) quand il merge.
    if let Some(ref t) = task
        && let Err(e) = update_verdict_merge_metadata(
            db,
            &t.id,
            &t.metadata,
            pr.number,
            &pr_url,
            "merge_ready_signaled",
        )
        .await
    {
        warn!(error = %e, task_id = %t.id, "Failed to update task metadata after merge-ready signal");
    }

    if let Err(e) = db
        .log_audit_event(
            session_id,
            "ci_success_merge_ready",
            &format!("pr:{}#{}@{}", event.repo, pr.number, pr.head_sha),
            Some("ci_green_verdict_pass"),
            Some("merge_ready_signaled"),
            Some(&format!(
                "trigger=check_suite_success reviewer={} task_id={task_id} pr_url={pr_url}",
                verdict_review.reviewer,
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log ci_success_merge_ready audit event");
    }

    info!(
        event = "ci_success_merge_ready",
        pr_number = pr.number,
        repo = %event.repo,
        head_sha = %pr.head_sha,
        task_id = %task_id,
        reviewer = %verdict_review.reviewer,
        "CI success handler: merge-ready signal emitted — the dispatcher owns the merge (mika#2248)"
    );

    VerdictAction::Handled {
        pre_digest: format_merge_ready_pre_digest(&event, &signal),
    }
}

// ---------------------------------------------------------------------------
// GitHub API helpers
// ---------------------------------------------------------------------------

/// Minimal PR info needed for the handler.
pub(crate) struct PrInfo {
    pub(crate) number: u64,
    pub(crate) head_sha: String,
}

/// Find an open PR whose head branch matches.
pub(crate) async fn find_open_pr(
    repo: &str,
    branch: &str,
    token: &str,
) -> Result<Option<PrInfo>, String> {
    let args = vec![
        "pr",
        "list",
        "--repo",
        repo,
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,headRefOid",
    ];

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh pr list timed out after 60s".to_string())??;

    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(None);
    }

    let prs: Vec<serde_json::Value> = serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse gh pr list output: {e}"))?;

    let first = match prs.first() {
        Some(p) => p,
        None => return Ok(None),
    };

    let number = first["number"]
        .as_u64()
        .ok_or_else(|| "PR number missing from gh pr list output".to_string())?;
    let head_sha = first["headRefOid"]
        .as_str()
        .ok_or_else(|| "headRefOid missing from gh pr list output".to_string())?
        .to_string();

    Ok(Some(PrInfo { number, head_sha }))
}

/// A qualifying review with VERDICT: pass.
struct PassVerdictReview {
    reviewer: String,
    commit_id: String,
}

/// Find the most recent APPROVED review with "VERDICT: pass" in its body.
async fn find_pass_verdict(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Option<PassVerdictReview>, String> {
    let api_path = format!("/repos/{repo}/pulls/{pr_number}/reviews");
    // Note: --paginate is intentionally omitted. gh api --paginate concatenates
    // raw JSON pages as `[...][...]` which is not valid JSON for serde_json.
    // Reviews are returned newest-last and GitHub defaults to 30 per page.
    // PRs with 30+ reviews are rare; if the pass verdict is beyond page 1,
    // the handler will return NoPassVerdict and the LLM handles follow-up.
    let args = vec!["api", &api_path];

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh api reviews timed out after 60s".to_string())??;

    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(None);
    }

    let reviews: Vec<serde_json::Value> = serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse reviews API output: {e}"))?;

    Ok(select_pass_verdict(&reviews))
}

/// The most recent APPROVED review whose body carries `VERDICT: pass` **and
/// whose author is the reviewer identity** (mika#2667 AC3).
///
/// Pure, so the read-side rule is testable without a network call. A `pass`
/// posted by any other login — the dispatcher identity included — is not a
/// verdict the merge path may consume: it is skipped, never selected, and an
/// older reviewer verdict still wins over a newer non-reviewer one. The login
/// is read by `forge_identity::is_reviewer_forge_login`, the same single reader
/// `verdict_handler` uses, so the two halves of the merge path agree.
fn select_pass_verdict(reviews: &[serde_json::Value]) -> Option<PassVerdictReview> {
    for review in reviews.iter().rev() {
        let state = review["state"].as_str().unwrap_or("");
        if state != "APPROVED" {
            continue;
        }

        let body = review["body"].as_str().unwrap_or("");
        // Reuse the canonical verdict parser to maintain contract parity
        // with verdict_handler (handles VERDICT:pass, VERDICT: pass, etc.)
        if parse_verdict(body) != Verdict::Pass {
            continue;
        }

        let Some(reviewer) = review["user"]["login"].as_str() else {
            continue;
        };
        if !mika_common::forge_identity::is_reviewer_forge_login(reviewer) {
            continue;
        }
        let commit_id = review["commit_id"].as_str().unwrap_or("").to_string();

        return Some(PassVerdictReview {
            reviewer: reviewer.to_string(),
            commit_id,
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

/// Update task metadata with a state on the `verdict_merge` ladder (mika#2248).
///
/// Shared with [`super::merge_ready_handler`] so the two halves of the CI-success
/// path write the same shape. The ladder has two rungs and they are not
/// interchangeable: `merge_ready_signaled` is written by the evaluator, which has
/// merged nothing; `merge_initiated` is written by the actor, and only after the
/// forge accepted the merge. Collapsing them is how a reader comes to believe a
/// PR was closed because a gate went green.
pub(super) async fn update_verdict_merge_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    pr_number: u64,
    pr_url: &str,
    state: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "verdict_merge": {
            "state": state,
            "trigger": "check_suite_success",
            "pr_number": pr_number,
            "pr_url": pr_url,
            "handled_at": crate::timestamp::now(),
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------

/// Format the pre-digest for an emitted merge-ready signal (mika#2248).
///
/// Two audiences in one text. The [`MergeReadySignal`] line is read by
/// [`super::merge_ready_handler`], which runs immediately after this handler in
/// the same turn and is the only code allowed to merge. The prose is read by the
/// LLM, and tells it the one thing it must not do — call the merge tool itself,
/// which would put the merge back under whichever agent happens to be running.
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses "signal" / "cleared" phrasing.
fn format_merge_ready_pre_digest(event: &CheckSuiteEvent, signal: &MergeReadySignal) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{} (branch: {})\n\
         CI checks all green + VERDICT: pass from @{} — every gate cleared for head {}.\n\n\
         {signal}\n\n\
         Task: {}\n\n\
         This handler does NOT merge (mika#2248): the dispatcher agent `{}` owns the merge and \
         acts on the signal above under its own identity, so the forge never records the \
         reviewer as the one who closed their own approval.\n\
         Do NOT call pr_merge_with_gate — you are not the merge actor on this path. \
         Update the task status to reflect the signal, then notify the user.\n\
         </ci_success_handler>",
        event.repo,
        signal.pr_number,
        event.branch,
        signal.reviewer_login,
        signal.head_sha,
        signal.task_id,
        mika_common::forge_identity::DISPATCHER_AGENT,
    )
}

/// Format the pre-digest for a merge error.
fn format_error_pre_digest(event: &CheckSuiteEvent, pr_number: u64, error: &str) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{} (branch: {})\n\
         CI checks all green — structural handler attempted merge but encountered an error.\n\n\
         Error: {error}\n\n\
         The structural merge handler could not finalize the merge. \
         Investigate the error and decide the next action.\n\
         </ci_success_handler>",
        event.repo, pr_number, event.branch
    )
}

/// Format the pre-digest for a DECISION-CORE hold (mika#1853 — forge-gate perimeter).
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses "blocked" / "held" / "must merge manually" phrasing.
fn format_decision_core_hold_pre_digest(
    event: &CheckSuiteEvent,
    pr_number: u64,
    reviewer: &str,
    perimeter_summary: &str,
) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{pr_number} (branch: {})\n\
         CI checks all green + VERDICT: pass from @{reviewer}, but this PR touches DECISION-CORE zone(s).\n\n\
         Auto-merge BLOCKED by forge-gate (mika#1853). {perimeter_summary}. Operator must merge manually.\n\n\
         Do NOT call pr_merge_with_gate for this PR — it will also block on the same gate. \
         Notify the operator that a manual review-and-merge is required.\n\
         </ci_success_handler>",
        event.repo, event.branch
    )
}

/// Format the enrichment message for a behind-main outcome (mika#2238).
///
/// `None` means the PR turned out not to be behind — the handler continues to
/// the merge path. Every other value ends the turn.
///
/// The remediation-specific half comes from
/// [`describe_behind_main_remediation`] so the do-not-merge instruction — the
/// point where a zealous LLM could re-open #1577 — has exactly one wording
/// shared with `verdict_handler`. The prior text ("Rebase the PR onto main
/// before merging") asked the LLM for an action the code now performs itself.
///
/// IMPORTANT: avoids the completion-claim guard's vocabulary (merged, deployed,
/// complete/completed, shipped) — pinned by the tests below.
fn format_behind_main_enrichment(
    remediation: &BehindMainRemediation,
    info: &BehindMainInfo,
) -> Option<String> {
    let described = describe_behind_main_remediation(remediation, info)?;
    Some(format!(
        "[ci_success_handler] All CI checks passed and VERDICT: pass exists, but {described}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify pre-digest messages don't trigger the completion-claim guard.
    // The guard regex: (?i)\b(merged|deployed|completed?|shipped)\b
    static COMPLETION_CLAIM_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?i)\b(merged|deployed|completed?|shipped)\b")
                .expect("completion claim regex")
        });

    fn sample_event() -> CheckSuiteEvent {
        CheckSuiteEvent {
            repo: "senara-solutions/mika".to_string(),
            branch: "feat/ci-success".to_string(),
        }
    }

    // -- Parser tests --

    #[test]
    fn parse_check_suite_success_valid() {
        let text =
            "[GitHub] Check suite success on senara-solutions/mika (branch: feat/ci-success)";
        let event = parse_check_suite_success(text).unwrap();
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.branch, "feat/ci-success");
    }

    #[test]
    fn parse_check_suite_success_with_trailing_text() {
        let text =
            "[GitHub] Check suite success on org/repo (branch: main)\n\nSome extra context here";
        let event = parse_check_suite_success(text).unwrap();
        assert_eq!(event.repo, "org/repo");
        assert_eq!(event.branch, "main");
    }

    #[test]
    fn parse_check_suite_failure_returns_none() {
        let text =
            "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/ci-success)";
        assert!(parse_check_suite_success(text).is_none());
    }

    #[test]
    fn parse_check_suite_non_matching_returns_none() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#522 by @mika-qa";
        assert!(parse_check_suite_success(text).is_none());
    }

    #[test]
    fn parse_check_suite_empty_returns_none() {
        assert!(parse_check_suite_success("").is_none());
    }

    // -- Pre-digest formatting tests --

    fn sample_signal() -> MergeReadySignal {
        MergeReadySignal {
            repo: "senara-solutions/mika".to_string(),
            pr_number: 571,
            head_sha: "abc123def".to_string(),
            branch: "feat/ci-success".to_string(),
            task_id: "task-123".to_string(),
            reviewer_login: "mika-platform-qa".to_string(),
        }
    }

    #[test]
    fn merge_ready_pre_digest_avoids_completion_claim_words() {
        let text = format_merge_ready_pre_digest(&sample_event(), &sample_signal());
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn merge_ready_pre_digest_contains_do_not_call_instruction() {
        let text = format_merge_ready_pre_digest(&sample_event(), &sample_signal());
        assert!(text.contains("Do NOT call pr_merge_with_gate"));
    }

    #[test]
    fn merge_ready_pre_digest_contains_task_id() {
        let text = format_merge_ready_pre_digest(&sample_event(), &sample_signal());
        assert!(text.contains("task-123"));
    }

    #[test]
    fn mika2248_le_pre_digest_porte_un_signal_relisible_par_lacteur() {
        // Le handoff passe par ce texte : s'il ne se reparse pas, l'acteur ne
        // voit rien et la PR reste ouverte. C'est la jointure à épingler.
        let signal = sample_signal();
        let text = format_merge_ready_pre_digest(&sample_event(), &signal);
        let parsed = mika_common::forge_identity::parse_merge_ready_signal(&text)
            .expect("le pré-digest doit porter un signal merge-ready relisible");
        assert_eq!(parsed, signal);
    }

    #[test]
    fn mika2248_le_pre_digest_nomme_le_dispatcher_comme_acteur() {
        let text = format_merge_ready_pre_digest(&sample_event(), &sample_signal());
        assert!(
            text.contains(mika_common::forge_identity::DISPATCHER_AGENT),
            "le pré-digest doit nommer qui merge, sinon le LLM comble le vide : {text}"
        );
    }

    // ---- DECISION-CORE hold pre-digest tests (mika#1853) ----

    #[test]
    fn decision_core_hold_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_decision_core_hold_pre_digest(
            &event,
            1851,
            "mika-qa",
            "DECISION-CORE (4 files: crates/mika-agent/src/task_engine/engine.rs, crates/mika-agent/src/task_engine/liveness.rs, crates/mika-agent/src/task_engine/mod.rs, crates/mika-agent/src/server/mod.rs)",
        );
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn decision_core_hold_pre_digest_contains_do_not_call_instruction() {
        let event = sample_event();
        let text = format_decision_core_hold_pre_digest(
            &event,
            1851,
            "mika-qa",
            "DECISION-CORE (1 file: foo.rs)",
        );
        assert!(text.contains("Do NOT call pr_merge_with_gate"));
    }

    #[test]
    fn decision_core_hold_pre_digest_contains_operator_manual_signal() {
        let event = sample_event();
        let text = format_decision_core_hold_pre_digest(
            &event,
            1851,
            "mika-qa",
            "DECISION-CORE (1 file: foo.rs)",
        );
        assert!(text.contains("Operator must merge manually"));
        assert!(text.contains("BLOCKED by forge-gate"));
        assert!(text.contains("mika#1853"));
    }

    #[test]
    fn decision_core_hold_pre_digest_contains_pr_number_and_reviewer() {
        let event = sample_event();
        let text = format_decision_core_hold_pre_digest(
            &event,
            1851,
            "mika-qa",
            "DECISION-CORE (1 file: foo.rs)",
        );
        assert!(text.contains("1851"));
        assert!(text.contains("mika-qa"));
    }

    #[test]
    fn error_pre_digest_contains_error_message() {
        let event = sample_event();
        let text = format_error_pre_digest(&event, 571, "Merge conflicts");
        assert!(text.contains("Merge conflicts"));
        assert!(text.contains("ci_success_handler"));
    }

    #[test]
    fn non_matching_text_returns_passthrough() {
        // This tests the parse path; the full handler test would need mocked subprocesses
        let text = "[GitHub] PR review (approved) on org/repo#42 by @reviewer";
        assert!(parse_check_suite_success(text).is_none());
    }

    // ---- Behind-main enrichment tests (#1577, mika#2238) ----

    fn behind_info() -> BehindMainInfo {
        BehindMainInfo {
            pr_base_sha: "abc1234deadbeef".to_string(),
            current_main_sha: "def5678cafebabe".to_string(),
        }
    }

    #[test]
    fn behind_main_enrichment_contains_both_shas() {
        let info = behind_info();
        let text = format_behind_main_enrichment(
            &BehindMainRemediation::Updated {
                observed_base_sha: "0bserved0bserved0bserved0bserved0bserved".to_string(),
            },
            &info,
        )
        .expect("an updated branch must still enrich the turn");
        assert!(
            text.contains(&info.pr_base_sha),
            "Enrichment missing pr_base_sha: {text}"
        );
        assert!(
            text.contains(&info.current_main_sha),
            "Enrichment missing current_main_sha: {text}"
        );
    }

    #[test]
    fn behind_main_enrichment_avoids_completion_claim_words() {
        // Every remediation shape is a pre-digest fed to an LLM turn, so every
        // one of them must clear the completion-claim guard — not just the
        // shape that happened to exist when the guard was written.
        for remediation in [
            BehindMainRemediation::Updated {
                observed_base_sha: "0bserved0bserved0bserved0bserved0bserved".to_string(),
            },
            BehindMainRemediation::AcceptedNotLanded,
            BehindMainRemediation::AlreadyAttempted,
            BehindMainRemediation::Conflict("merge conflict between base and head".to_string()),
            BehindMainRemediation::Failed("HTTP 403: Resource not accessible".to_string()),
            BehindMainRemediation::Contradiction("still behind".to_string()),
            BehindMainRemediation::BaseNotMain("release/1.4".to_string()),
        ] {
            let text = format_behind_main_enrichment(&remediation, &behind_info())
                .expect("only NotBehind yields no enrichment");
            assert!(
                !COMPLETION_CLAIM_RE.is_match(&text),
                "Behind-main enrichment for {remediation:?} contains a completion-claim \
                 trigger word: {text}"
            );
        }
    }

    #[test]
    fn mika2238_no_enrichment_ever_tells_the_agent_to_rebase_by_hand() {
        // The old wording ("Rebase the PR onto main before merging") is the one
        // the prompts now forbid in the same breath — a webhook turn cannot
        // reach the worktree, so an agent that obeys it either fails or edits
        // its own sandbox. Every shape must instead route to the operator.
        for remediation in [
            BehindMainRemediation::Updated {
                observed_base_sha: "0bserved0bserved0bserved0bserved0bserved".to_string(),
            },
            BehindMainRemediation::AcceptedNotLanded,
            BehindMainRemediation::AlreadyAttempted,
            BehindMainRemediation::Conflict("merge conflict".to_string()),
            BehindMainRemediation::Failed("gh exit code 1".to_string()),
            BehindMainRemediation::Contradiction("still behind".to_string()),
            BehindMainRemediation::BaseNotMain("release/1.4".to_string()),
        ] {
            let text = format_behind_main_enrichment(&remediation, &behind_info())
                .expect("only NotBehind yields no enrichment");
            assert!(
                !text.contains("Rebase the PR onto main"),
                "enrichment for {remediation:?} still instructs a hand rebase the prompts \
                 forbid: {text}"
            );
        }
    }

    #[test]
    fn mika2238_a_failed_update_names_the_credential_remedy() {
        // The webhook paths never reach `classify_credential_scope_error` — they
        // render prose, not a `MergeGateResult` — so the remedy mika#1616 exists
        // to state has to be in the text, or a 403 is named nowhere the agent
        // can act on.
        let text = format_behind_main_enrichment(
            &BehindMainRemediation::Failed(
                "gh: Resource not accessible by integration (HTTP 403)".to_string(),
            ),
            &behind_info(),
        )
        .expect("a failed update must enrich the turn");
        assert!(
            text.contains("install the mika GitHub App"),
            "a failed update must carry the credential remedy: {text}"
        );
    }

    #[test]
    fn mika2238_the_updated_enrichment_admits_the_pr_needs_a_fresh_review() {
        // The stale-SHA gate at step 4 compares the QA review's commit_id with
        // the PR head. An update-branch moves that head, so the approval no
        // longer matches and this handler will hold the PR on the next pass.
        // Saying "the webhook finishes the merge" would be a promise the gate
        // right above refuses to keep.
        let text = format_behind_main_enrichment(
            &BehindMainRemediation::Updated {
                observed_base_sha: "0bserved0bserved0bserved0bserved0bserved".to_string(),
            },
            &behind_info(),
        )
        .expect("an updated branch must enrich the turn");
        assert!(
            text.contains("fresh QA review"),
            "the updated enrichment must say the PR returns to QA: {text}"
        );
    }

    #[test]
    fn mika2238_updated_enrichment_forbids_merging_in_this_turn() {
        // R3, handler side. An update-branch creates a new head commit that no
        // CI run has validated; an LLM that merges it anyway re-opens #1577
        // through the door mika#2238 opened.
        let text = format_behind_main_enrichment(
            &BehindMainRemediation::Updated {
                observed_base_sha: "0bserved0bserved0bserved0bserved0bserved".to_string(),
            },
            &behind_info(),
        )
        .expect("an updated branch must enrich the turn");
        assert!(
            text.contains("Do NOT merge"),
            "Updated enrichment must forbid merging in this turn: {text}"
        );
        assert!(
            text.contains("pr_merge_with_gate"),
            "Updated enrichment must name the tool not to call: {text}"
        );
    }

    #[test]
    fn mika2238_not_behind_yields_no_enrichment_so_the_handler_continues() {
        // `None` is the only value that lets the handler reach the signal step.
        assert!(
            format_behind_main_enrichment(&BehindMainRemediation::NotBehind, &behind_info())
                .is_none(),
            "a PR that is not behind must not be held by this path"
        );
    }

    // ---- Audit-durable dedup gate tests (AC2, mika#1869) ----

    #[test]
    fn is_duplicate_processed_matches_ticket_semantics() {
        // Zero prior markers → first real processing (not a duplicate).
        assert!(!is_duplicate_processed(0));
        // One or more prior markers → cross-restart duplicate.
        assert!(is_duplicate_processed(1));
        assert!(is_duplicate_processed(7));
    }

    #[tokio::test]
    async fn audit_gate_query_wiring_detects_prior_marker() {
        use crate::db::Database;

        let db = AsyncDatabase::new(Database::open_in_memory().unwrap());
        let session_id = "test-session";
        db.create_session(session_id, "mika", "github")
            .await
            .unwrap();

        let processed_key = "pr:senara-solutions/mika#1869@abc123";
        let since = crate::timestamp::now_minus(chrono::Duration::seconds(60));

        // Before any marker: count is 0 → not a duplicate → handler would proceed.
        let count_before = db
            .count_recent_audit_events_for_target(
                "ci_success_handler_processed",
                processed_key,
                &since,
            )
            .await
            .unwrap();
        assert_eq!(count_before, 0);
        assert!(!is_duplicate_processed(count_before));

        // Write the processing marker (the handler's first-processing path).
        db.log_audit_event(
            session_id,
            "ci_success_handler_processed",
            processed_key,
            None,
            None,
            Some("trigger=check_suite_success dedup_marker"),
            Some("trace-1869"),
        )
        .await
        .unwrap();

        // After the marker: the exact-match count query sees it → duplicate → skip.
        let count_after = db
            .count_recent_audit_events_for_target(
                "ci_success_handler_processed",
                processed_key,
                &since,
            )
            .await
            .unwrap();
        assert_eq!(count_after, 1);
        assert!(is_duplicate_processed(count_after));

        // A different head_sha key is unaffected (distinct-push invariant is
        // head_sha-precise even at the audit layer).
        let other_key = "pr:senara-solutions/mika#1869@def456";
        let count_other = db
            .count_recent_audit_events_for_target("ci_success_handler_processed", other_key, &since)
            .await
            .unwrap();
        assert_eq!(count_other, 0);
        assert!(!is_duplicate_processed(count_other));
    }
}

#[cfg(test)]
mod mika2667_tests {
    use super::select_pass_verdict;
    use serde_json::json;

    fn review(login: &str, state: &str, body: &str, commit: &str) -> serde_json::Value {
        json!({"user": {"login": login}, "state": state, "body": body, "commit_id": commit})
    }

    /// The `check_suite` half of the merge path honours the same read-side
    /// rule as `verdict_handler`: an approved `pass` from the dispatcher
    /// identity is never selected.
    #[test]
    fn a_pass_from_the_dispatcher_identity_is_never_selected() {
        let reviews = vec![review(
            "mika-platform-dev",
            "APPROVED",
            "VERDICT: pass",
            "abc",
        )];
        assert!(select_pass_verdict(&reviews).is_none());
    }

    /// Positive control, and ordering: the reviewer's own verdict is selected,
    /// even when a newer non-reviewer `pass` sits after it.
    #[test]
    fn the_reviewer_verdict_is_selected_over_a_newer_non_reviewer_one() {
        let reviews = vec![
            review("mika-platform-qa", "APPROVED", "VERDICT: pass", "old"),
            review("mika-platform-dev", "APPROVED", "VERDICT: pass", "new"),
        ];
        let selected = select_pass_verdict(&reviews).expect("the reviewer verdict");
        assert_eq!(selected.reviewer, "mika-platform-qa");
        assert_eq!(selected.commit_id, "old");
    }

    /// A review with no readable author is not a reviewer verdict.
    #[test]
    fn a_review_with_no_author_is_not_selected() {
        let reviews = vec![json!({"state": "APPROVED", "body": "VERDICT: pass", "commit_id": "x"})];
        assert!(select_pass_verdict(&reviews).is_none());
    }

    /// mika#2675 phase 3 — seul `Delivered` atteste une notification ; un
    /// canal absent, un échec ou une erreur n'en sont pas une.
    #[test]
    fn mika2675_seul_delivered_atteste_la_notification() {
        use crate::messaging::SendOutcome;
        use crate::server::ci_success_handler::{
            DECISION_CORE_HOLD_NOT_NOTIFIED, DECISION_CORE_HOLD_NOTIFIED, DECISION_CORE_HOLD_TOOL,
            decision_core_notification_value, head_key,
        };
        let value = |o: anyhow::Result<SendOutcome>| decision_core_notification_value(&o).0;
        assert_eq!(
            value(Ok(SendOutcome::Delivered)),
            DECISION_CORE_HOLD_NOTIFIED
        );
        for o in [
            Ok(SendOutcome::Failed {
                reason: "503".into(),
            }),
            Ok(SendOutcome::NoChannel),
            Err(anyhow::anyhow!("réseau")),
        ] {
            assert_eq!(value(o), DECISION_CORE_HOLD_NOT_NOTIFIED);
        }
        assert_eq!(
            DECISION_CORE_HOLD_TOOL,
            "ci_success_handler_decision_core_hold"
        );
        assert_eq!(
            head_key("senara-solutions/mika", 2678, "abc"),
            "pr:senara-solutions/mika#2678@abc"
        );
    }
}
