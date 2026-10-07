//! Pré-filtre déterministe des événements webhook sans suite possible
//! (mika#2675, phase 1).
//!
//! Mesuré le 2026-10-07 : une `check_suite` verte sur `main` coûte deux tours
//! LLM (mika-dev et mika-qa, ≈ 100 k tokens d'entrée à eux deux) qui concluent
//! « CI green on main. No action », et un label sans effet coûte un tour
//! mika-dev qui conclut « pas de `ready`, pas de dispatch ». Le moteur sait
//! les deux choses sans modèle.
//!
//! # Placement : après les handlers déterministes, avant `run_agent`
//!
//! Les handlers structurels gardent tous leurs effets (évaluation de merge,
//! notification, nettoyage de lignes). Ce module ne change pas ce que le moteur
//! FAIT, seulement si l'on demande ENSUITE au modèle. Les deux classes de la
//! phase 1 exigent en plus un texte intact : un handler qui a remplacé ou
//! enrichi `req.text` a quelque chose à dire au modèle, et le tour a lieu.
//!
//! # Fail-safe vers le LLM
//!
//! Interrupteur désarmé, texte touché, recherche d'état en erreur : le tour a
//! lieu comme avant. On paie, on ne perd rien.
//!
//! # Phase 2 : la classe (c), `pr_review` non actionnable sans tâche
//!
//! Le `verdict_handler` sait déjà, en code, qu'aucune tâche n'est active pour
//! la PR : il notifie l'opérateur d'un `hold[review]` (mika#2667 AC2), refuse le
//! `pass` d'une identité qui n'est pas le relecteur (mika#2667 AC3), traite une
//! ligne VERDICT illisible en `hold` par défaut — puis rend quand même la main
//! au modèle, qui conclut « stale — no action ». Ces chemins enrichissent ou
//! remplacent TOUJOURS le texte, donc la règle « texte intact » de la phase 1
//! les excluait par construction. Pour (c), le texte doit être intact APRÈS le
//! `verdict_handler` : lui a déjà dit au modèle ce qu'il avait à dire, et ce
//! n'est rien d'actionnable ; un handler ultérieur qui toucherait le texte
//! garde le tour.
//!
//! Une revue SANS ligne `VERDICT:` n'est pas un verdict : c'est la forme d'une
//! consigne écrite en revue, et l'identité GitHub de l'opérateur est partagée
//! (AC2, « un commentaire n'est jamais filtré »). Elle n'entre pas dans (c).
//!
//! # Phase 3 : la classe (b), `check_suite` verte sur une tête déjà décidée
//!
//! Mesuré le 2026-10-07 : les traitements d'une même tête sont espacés de
//! plusieurs minutes, donc au-delà de la dédup de 60 s de `ci_success_handler`.
//! Une `check_suite` tardive retraite la tête, redécide DECISION-CORE, renotifie
//! l'opérateur, et le tour LLM qui suit conclut « Already notified ». Les
//! retours de dédup (étapes 2b/2c) ne portent, eux, aucune trace de ce que la
//! tête a décidé : ils gardent leur tour.
//!
//! Trois termes, tous lus dans une trace durable, et la fonction pure
//! [`head_decided_and_notified`] les porte tous :
//! - **décision attestée** — CET événement a écrit l'attestation
//!   `ci_success_handler_decision_core_hold` (le marqueur `processed`, écrit
//!   avant l'évaluation, ne suffit pas) ;
//! - **même tête** — une attestation d'un AUTRE événement porte exactement la
//!   même cible `pr:{repo}#{n}@{sha}` ;
//! - **notification attestée** — cette attestation antérieure dit `notified`.
//!
//! Le texte ne doit avoir été touché que par `ci_success_handler`, comme (c)
//! l'exige du `verdict_handler`. Une variante et ses faits, pas un second
//! mécanisme.

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;

use super::ci_success_handler::{
    DECISION_CORE_HOLD_NOTIFIED, DECISION_CORE_HOLD_TOOL, parse_check_suite_success,
};
use super::verdict::{Verdict, parse_pr_review_event, parse_verdict, verdict_raw_value};
use super::webhook_queue_v2::{WebhookEventKind, classify_event};

/// Kill-switch, armé par défaut. Même table de vérité que les interrupteurs
/// de mika#2671 ([`crate::qa_head_supersession::parse_switch`]).
pub(crate) const PREFILTER_ENV: &str = "MIKA_WEBHOOK_PREFILTER";

/// `audit_events.tool_name` d'un événement écarté. **SOLE WRITER** :
/// [`skip_turn`].
pub(crate) const PREFILTER_SKIPPED_TOOL: &str = "webhook_prefilter_skipped";

/// La branche par défaut des dépôts que la boucle sert.
pub(crate) const DEFAULT_BRANCH: &str = "main";

/// Délai de la lecture de l'état de PR côté forge. Au-delà, l'état est
/// illisible et le tour a lieu (AC3).
const PR_STATE_TIMEOUT_SECS: u64 = 15;

/// Une classe d'événement écartée sans LLM. Valeur de `after_value` dans
/// l'audit : un format de fil, épinglé par test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefilterClass {
    /// (a) `check_suite` verte sur la branche par défaut, sans tâche active.
    GreenCheckSuiteDefaultBranch,
    /// (d) label qu'aucune partie de la boucle ne lit.
    InertLabel,
    /// (c) `hold[review]` sans tâche : `hold_review_without_task` l'a déjà
    /// remis à l'opérateur (notification + ligne d'audit).
    VerdictHoldTracked,
    /// (c) `pass` d'une identité qui n'est pas le relecteur QA.
    VerdictNonReviewer,
    /// (c) ligne `VERDICT:` présente mais illisible.
    VerdictUnreadable,
    /// (c) PR fermée ou mergée côté forge.
    VerdictPrClosed,
    /// (b) `check_suite` verte sur une tête de PR déjà décidée DECISION-CORE
    /// et notifiée, que cet événement redécide à l'identique.
    GreenCheckSuiteHeadDecided,
}

impl PrefilterClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::GreenCheckSuiteDefaultBranch => "green_check_suite_default_branch",
            Self::InertLabel => "inert_label",
            Self::VerdictHoldTracked => "verdict_hold_tracked",
            Self::VerdictNonReviewer => "verdict_non_reviewer",
            Self::VerdictUnreadable => "verdict_unreadable",
            Self::VerdictPrClosed => "verdict_pr_closed",
            Self::GreenCheckSuiteHeadDecided => "green_check_suite_head_decided",
        }
    }
}

/// Ce que le texte dit d'un verdict, avant consultation de l'état. Chaque
/// champ est un terme de « non actionnable » ; aucun ne suffit sans l'absence
/// de tâche active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewVerdictFacts {
    pub(crate) repo: String,
    pub(crate) pr_number: u64,
    pub(crate) pr_url: String,
    /// `hold[review]` — le seul `hold` que le handler remet à l'opérateur.
    /// Un `hold[x]` inconnu n'est suivi par personne et garde son tour.
    pub(crate) hold_tracked: bool,
    /// `pass` dont l'auteur n'est pas `REVIEWER_FORGE_LOGIN` : le seul verdict
    /// de non-relecteur que le handler referme (refus + audit, mika#2667 AC3).
    /// Un `block[*]` ou un `hold[x]` d'une autre identité n'est notifié à
    /// personne — l'identité de l'opérateur est partagée — et garde son tour.
    pub(crate) non_reviewer_pass: bool,
    /// Ligne `VERDICT:` présente, valeur non reconnue.
    pub(crate) unreadable: bool,
}

/// Un événement qui POURRAIT être écarté, avant consultation de l'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Candidate {
    GreenDefaultBranch {
        repo: String,
        branch: String,
    },
    /// (b) toute `check_suite` verte hors branche par défaut. Elle n'est
    /// écartée qu'au terme de [`decide`], sur la trace laissée par le handler.
    GreenPrBranch {
        repo: String,
        branch: String,
    },
    InertLabel {
        repo: String,
        issue: u64,
        label: String,
    },
    /// (c) toute revue portant une ligne `VERDICT:`. Le verdict n'est
    /// écarté qu'au terme de [`decide`].
    ReviewVerdict(ReviewVerdictFacts),
}

/// Fonction pure : classe le texte du gateway. `None` = hors population, le
/// tour a lieu. Lit les grammaires existantes, n'en écrit aucune.
pub(crate) fn classify(text: &str) -> Option<Candidate> {
    if let Some(event) = parse_check_suite_success(text) {
        return Some(if event.branch == DEFAULT_BRANCH {
            Candidate::GreenDefaultBranch {
                repo: event.repo,
                branch: event.branch,
            }
        } else {
            Candidate::GreenPrBranch {
                repo: event.repo,
                branch: event.branch,
            }
        });
    }
    if let Some(event) = parse_pr_review_event(text) {
        // Pas de ligne VERDICT : une revue-consigne, jamais filtrée (AC2).
        verdict_raw_value(&event.body)?;
        let verdict = parse_verdict(&event.body);
        let pr_url = event.pr_url();
        return Some(Candidate::ReviewVerdict(ReviewVerdictFacts {
            hold_tracked: matches!(&verdict, Verdict::Hold(r) if r.eq_ignore_ascii_case("review")),
            non_reviewer_pass: matches!(verdict, Verdict::Pass)
                && !mika_common::forge_identity::is_reviewer_forge_login(&event.reviewer),
            unreadable: matches!(verdict, Verdict::Missing { .. }),
            repo: event.repo,
            pr_number: event.pr_number,
            pr_url,
        }));
    }
    match classify_event(text) {
        WebhookEventKind::IssueLabeled { repo, issue, label }
            if !crate::webhook_dispatch::label_read_by_loop(&label) =>
        {
            Some(Candidate::InertLabel { repo, issue, label })
        }
        _ => None,
    }
}

/// L'état d'une PR côté forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrForgeState {
    Open,
    Closed,
    Merged,
}

/// Lit `gh pr view --json state`. Toute autre forme est une erreur : un état
/// inconnu est illisible, jamais « ouvert » ni « fermé » (AC3).
pub(crate) fn parse_pr_state(json: &str) -> anyhow::Result<PrForgeState> {
    let value: serde_json::Value = serde_json::from_str(json.trim())?;
    match value.get("state").and_then(|s| s.as_str()) {
        Some("OPEN") => Ok(PrForgeState::Open),
        Some("CLOSED") => Ok(PrForgeState::Closed),
        Some("MERGED") => Ok(PrForgeState::Merged),
        other => anyhow::bail!("état de PR non reconnu : {other:?}"),
    }
}

/// Une ligne d'audit, réduite à ce que la classe (b) lit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditFact {
    /// Rang d'écriture (`audit_events.id`) : seule une attestation strictement
    /// ANTÉRIEURE à celle de cet événement vaut « déjà décidée ».
    pub(crate) id: i64,
    pub(crate) tool: String,
    pub(crate) target: String,
    pub(crate) after: Option<String>,
    pub(crate) trace: Option<String>,
}

impl From<crate::db::AuditEvent> for AuditFact {
    fn from(e: crate::db::AuditEvent) -> Self {
        Self {
            id: e.id,
            tool: e.tool_name,
            target: e.target_key,
            after: e.after_value,
            trace: e.trace_id,
        }
    }
}

/// Au plus autant d'attestations antérieures lues pour une PR.
const PRIOR_HOLDS_LIMIT: u32 = 50;

/// Les recherches d'état du pré-filtre, injectées pour que les tests les
/// bouchonnent : aucun appel GitHub réel en test.
pub(crate) trait PrefilterState {
    async fn branch_has_active_task(&self, branch: &str) -> anyhow::Result<bool>;
    async fn pr_has_active_task(&self, pr_url: &str) -> anyhow::Result<bool>;
    async fn pr_state(&self, repo: &str, pr_number: u64) -> anyhow::Result<PrForgeState>;
    /// (b) les lignes d'audit écrites pour CET événement (son `trace_id`).
    async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>>;
    /// (b) les attestations DECISION-CORE dont la cible commence par
    /// `pr_prefix` (`pr:{repo}#{n}@`), tous événements confondus.
    async fn decision_core_holds(&self, pr_prefix: &str) -> anyhow::Result<Vec<AuditFact>>;
}

/// L'état réel : base de l'agent, et `gh` pour la forge.
pub(crate) struct LiveState<'a> {
    pub(crate) db: &'a AsyncDatabase,
    pub(crate) github_token: Option<&'a str>,
    /// L'identité de l'événement, que les handlers posent en `trace_id`.
    pub(crate) request_id: &'a str,
}

impl PrefilterState for LiveState<'_> {
    async fn branch_has_active_task(&self, branch: &str) -> anyhow::Result<bool> {
        Ok(self.db.find_active_task_by_branch(branch).await?.is_some())
    }

    async fn pr_has_active_task(&self, pr_url: &str) -> anyhow::Result<bool> {
        Ok(self.db.find_active_task_by_pr_url(pr_url).await?.is_some())
    }

    async fn pr_state(&self, repo: &str, pr_number: u64) -> anyhow::Result<PrForgeState> {
        let Some(token) = self.github_token else {
            anyhow::bail!("aucun jeton GitHub résolu");
        };
        let number = pr_number.to_string();
        let args = ["pr", "view", &number, "--repo", repo, "--json", "state"];
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(PR_STATE_TIMEOUT_SECS),
            crate::tools::pr_merge_with_gate::run_gh_subprocess(&args, token),
        )
        .await
        .map_err(|_| anyhow::anyhow!("gh pr view : délai de {PR_STATE_TIMEOUT_SECS} s dépassé"))?
        .map_err(|e| anyhow::anyhow!(e))?;
        parse_pr_state(&output)
    }

    async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>> {
        let rows = self
            .db
            .get_audit_events_by_trace_ids(vec![self.request_id.to_string()])
            .await?;
        Ok(rows.into_iter().map(AuditFact::from).collect())
    }

    async fn decision_core_holds(&self, pr_prefix: &str) -> anyhow::Result<Vec<AuditFact>> {
        let rows = self
            .db
            .get_audit_events_for_target_prefix(
                DECISION_CORE_HOLD_TOOL,
                pr_prefix,
                PRIOR_HOLDS_LIMIT,
            )
            .await?;
        Ok(rows.into_iter().map(AuditFact::from).collect())
    }
}

/// (b) Fonction pure, trois termes conjonctifs. Rend la tête quand CET
/// événement a redécidé DECISION-CORE sur `repo` (décision attestée) et qu'un
/// AUTRE événement l'avait déjà décidée AVANT lui sur la même cible (même
/// tête) et notifiée (notification attestée). Tout le reste garde son tour —
/// en particulier le premier traitement d'une tête, même si un traitement plus
/// tardif a écrit son attestation avant que le premier n'atteigne la porte.
pub(crate) fn head_decided_and_notified(
    repo: &str,
    this_event: &[AuditFact],
    prior: &[AuditFact],
) -> Option<String> {
    let mine = this_event
        .iter()
        .find(|r| r.tool == DECISION_CORE_HOLD_TOOL)?;
    if !mine.target.starts_with(&format!("pr:{repo}#")) {
        return None;
    }
    prior
        .iter()
        .any(|p| {
            p.tool == DECISION_CORE_HOLD_TOOL
                && p.trace != mine.trace
                && p.id < mine.id
                && p.target == mine.target
                && p.after.as_deref() == Some(DECISION_CORE_HOLD_NOTIFIED)
        })
        .then(|| mine.target.clone())
}

/// `pr:{repo}#{n}@{sha}` → `pr:{repo}#{n}@`. `None` si la cible n'a pas de SHA.
fn pr_prefix(head: &str) -> Option<&str> {
    head.rfind('@').map(|i| &head[..=i])
}

/// L'issue de la décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Llm,
    Skip {
        class: PrefilterClass,
        target: String,
    },
}

fn state_unreadable(what: &str, target: &str, e: &anyhow::Error) -> Decision {
    warn!(
        event = "webhook_prefilter_state_unreadable",
        what,
        target = %target,
        error = %e,
        "pré-filtre : état illisible — le tour a lieu (fail-safe)"
    );
    Decision::Llm
}

/// Décide un candidat. Toute erreur de recherche laisse passer (AC3).
pub(crate) async fn decide(candidate: Candidate, state: &impl PrefilterState) -> Decision {
    match candidate {
        Candidate::GreenDefaultBranch { repo, branch } => {
            let target = format!("check_suite:{repo}@{branch}");
            match state.branch_has_active_task(&branch).await {
                Ok(false) => Decision::Skip {
                    class: PrefilterClass::GreenCheckSuiteDefaultBranch,
                    target,
                },
                Ok(true) => Decision::Llm,
                Err(e) => state_unreadable("branch_task", &target, &e),
            }
        }
        Candidate::GreenPrBranch { repo, branch } => {
            decide_head_decided(&repo, &branch, state).await
        }
        Candidate::InertLabel { repo, issue, .. } => Decision::Skip {
            class: PrefilterClass::InertLabel,
            target: format!("issue:{repo}#{issue}"),
        },
        Candidate::ReviewVerdict(facts) => decide_review_verdict(facts, state).await,
    }
}

/// (b) : la trace de CET événement d'abord (index sur `trace_id`, presque
/// toujours sans attestation), les attestations de la PR ensuite. Toute lecture
/// en erreur laisse passer (AC3).
async fn decide_head_decided(repo: &str, branch: &str, state: &impl PrefilterState) -> Decision {
    let pending = format!("check_suite:{repo}@{branch}");
    let this_event = match state.event_audit_rows().await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("event_audit", &pending, &e),
    };
    let Some(prefix) = this_event
        .iter()
        .find(|r| r.tool == DECISION_CORE_HOLD_TOOL)
        .and_then(|r| pr_prefix(&r.target))
    else {
        return Decision::Llm;
    };
    let prior = match state.decision_core_holds(prefix).await {
        Ok(rows) => rows,
        Err(e) => return state_unreadable("decision_core_holds", prefix, &e),
    };
    match head_decided_and_notified(repo, &this_event, &prior) {
        Some(head) => Decision::Skip {
            class: PrefilterClass::GreenCheckSuiteHeadDecided,
            target: format!("check_suite:{}", head.strip_prefix("pr:").unwrap_or(&head)),
        },
        None => Decision::Llm,
    }
}

/// (c) : sans tâche active, ET l'un des quatre termes de « non actionnable ».
/// Les trois termes lus dans le texte passent avant la forge, pour ne payer
/// `gh` qu'au besoin.
async fn decide_review_verdict(facts: ReviewVerdictFacts, state: &impl PrefilterState) -> Decision {
    let target = format!("pr_review:{}#{}", facts.repo, facts.pr_number);
    match state.pr_has_active_task(&facts.pr_url).await {
        Ok(false) => {}
        Ok(true) => return Decision::Llm,
        Err(e) => return state_unreadable("pr_task", &target, &e),
    }
    let class = if facts.hold_tracked {
        PrefilterClass::VerdictHoldTracked
    } else if facts.non_reviewer_pass {
        PrefilterClass::VerdictNonReviewer
    } else if facts.unreadable {
        PrefilterClass::VerdictUnreadable
    } else {
        match state.pr_state(&facts.repo, facts.pr_number).await {
            Ok(PrForgeState::Closed | PrForgeState::Merged) => PrefilterClass::VerdictPrClosed,
            Ok(PrForgeState::Open) => return Decision::Llm,
            Err(e) => return state_unreadable("pr_state", &target, &e),
        }
    };
    Decision::Skip { class, target }
}

/// L'interrupteur est-il armé ? Lu à chaque événement.
pub(crate) fn prefilter_enabled() -> bool {
    crate::qa_head_supersession::parse_switch(
        PREFILTER_ENV,
        std::env::var(PREFILTER_ENV).ok().as_deref(),
    )
}

/// Ce que les handlers ont fait du texte. (a) et (d) exigent un texte
/// intact ; (c) exige un texte intact APRÈS le `verdict_handler` ; (b) exige
/// qu'aucun handler autre que `ci_success_handler` n'y ait touché.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Touched {
    pub(crate) by_any_handler: bool,
    pub(crate) after_verdict_handler: bool,
    pub(crate) outside_ci_success_handler: bool,
}

impl Touched {
    fn applies_to(self, candidate: &Candidate) -> bool {
        match candidate {
            Candidate::ReviewVerdict(_) => self.after_verdict_handler,
            Candidate::GreenPrBranch { .. } => self.outside_ci_success_handler,
            Candidate::GreenDefaultBranch { .. } | Candidate::InertLabel { .. } => {
                self.by_any_handler
            }
        }
    }
}

/// La porte. Rend `true` quand le tour LLM est écarté ; une ligne `info!` et
/// une ligne d'audit nommées sont alors écrites.
pub(crate) async fn skip_turn(
    db: &AsyncDatabase,
    state: &impl PrefilterState,
    original_text: &str,
    touched: Touched,
    request_id: &str,
    enabled: bool,
) -> bool {
    if !enabled {
        return false;
    }
    let Some(candidate) = classify(original_text) else {
        return false;
    };
    if touched.applies_to(&candidate) {
        return false;
    }
    let Decision::Skip { class, target } = decide(candidate, state).await else {
        return false;
    };

    info!(
        event = PREFILTER_SKIPPED_TOOL,
        agent_id = %db.agent_id(),
        class = class.as_str(),
        target = %target,
        request_id,
        "événement sans suite possible — aucun tour LLM (mika#2675)"
    );
    if let Err(e) = db
        .log_audit_event(
            "system",
            PREFILTER_SKIPPED_TOOL,
            &target,
            None,
            Some(class.as_str()),
            Some(&format!("request_id={request_id}")),
            Some(request_id),
        )
        .await
    {
        warn!(
            event = "webhook_prefilter_audit_failed",
            target = %target,
            error = %e,
            "événement écarté mais ligne d'audit non écrite"
        );
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const GREEN_MAIN: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: main)";
    const INERT: &str = "[GitHub] Issue labeled p1-important on senara-solutions/mika#2675 — fix(webhooks): pré-filtre\nhttps://github.com/senara-solutions/mika/issues/2675\nLabeled by: @samidarko";

    fn labeled(label: &str) -> String {
        format!("[GitHub] Issue labeled {label} on senara-solutions/mika#2675 — t\nhttps://x")
    }

    #[test]
    fn mika2675_classify_reconnait_les_deux_classes() {
        assert_eq!(
            classify(GREEN_MAIN),
            Some(Candidate::GreenDefaultBranch {
                repo: "senara-solutions/mika".into(),
                branch: "main".into()
            })
        );
        assert_eq!(
            classify(INERT),
            Some(Candidate::InertLabel {
                repo: "senara-solutions/mika".into(),
                issue: 2675,
                label: "p1-important".into()
            })
        );
    }

    /// (a) terme par terme : conclusion, branche, préfixe. Une branche autre
    /// que `main` sort de (a) et entre dans (b) ; un échec, un `timed_out` ou
    /// un texte sans préfixe ne sont candidats ni à l'une ni à l'autre (AC2).
    #[test]
    fn mika2675_classify_a_chaque_terme_sort_de_la_population() {
        for text in [
            "[GitHub] Check suite failure on senara-solutions/mika (branch: main)",
            "[GitHub] Check suite timed_out on senara-solutions/mika (branch: main)",
            "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/2675/x)",
            "[GitHub] Check suite timed_out on senara-solutions/mika (branch: fix/2675/x)",
            "Check suite success on senara-solutions/mika (branch: main)",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
        for (text, branch) in [
            (
                "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)",
                "fix/2675/x",
            ),
            (
                "[GitHub] Check suite success on senara-solutions/mika (branch: mainline)",
                "mainline",
            ),
        ] {
            assert_eq!(
                classify(text),
                Some(Candidate::GreenPrBranch {
                    repo: "senara-solutions/mika".into(),
                    branch: branch.into()
                }),
                "{text}"
            );
        }
    }

    /// (d) chaque famille de l'ensemble lu par la boucle sort de la population.
    #[test]
    fn mika2675_classify_d_chaque_label_lu_sort_de_la_population() {
        for label in [
            "ready",
            "Ready",
            "blocked",
            "operator-review",
            "operator-gated",
            "operator-iterate",
            "dispatch:loop",
            "dispatch:ssc",
            "loop-substrate",
            "needs-build",
            "wip-rescue",
            "phase:2",
            "origin:loop",
        ] {
            assert_eq!(classify(&labeled(label)), None, "{label}");
        }
        for label in ["bug", "p2-normal", "enhancement", "help wanted"] {
            assert!(classify(&labeled(label)).is_some(), "{label}");
        }
    }

    #[test]
    fn mika2675_classify_hors_population() {
        for text in [
            "[GitHub] PR opened: senara-solutions/mika#1 — t (branch: main)\nu",
            "[GitHub] New comment on senara-solutions/mika#1 (t) by @samidarko\nu\n\nfais-le",
            "[GitHub] Issue labeled: senara-solutions/mika#1 — t\nu",
            "bonjour",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }

    /// État bouchonné. `None` = la recherche ne doit pas être consultée :
    /// elle panique si elle l'est. Aucun appel GitHub réel.
    #[derive(Default)]
    pub(crate) struct Stub {
        pub(crate) branch_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_state: Option<Result<PrForgeState, &'static str>>,
        pub(crate) event_rows: Option<Result<Vec<AuditFact>, &'static str>>,
        pub(crate) holds: Option<Result<Vec<AuditFact>, &'static str>>,
    }

    fn answer_rows(
        slot: &Option<Result<Vec<AuditFact>, &'static str>>,
        what: &str,
    ) -> anyhow::Result<Vec<AuditFact>> {
        match slot {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(anyhow::anyhow!(*e)),
            None => panic!("recherche d'état non attendue : {what}"),
        }
    }

    fn answer<T: Copy>(slot: &Option<Result<T, &'static str>>, what: &str) -> anyhow::Result<T> {
        match slot {
            Some(Ok(v)) => Ok(*v),
            Some(Err(e)) => Err(anyhow::anyhow!(*e)),
            None => panic!("recherche d'état non attendue : {what}"),
        }
    }

    impl PrefilterState for Stub {
        async fn branch_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
            answer(&self.branch_task, "branch_task")
        }
        async fn pr_has_active_task(&self, _: &str) -> anyhow::Result<bool> {
            answer(&self.pr_task, "pr_task")
        }
        async fn pr_state(&self, _: &str, _: u64) -> anyhow::Result<PrForgeState> {
            answer(&self.pr_state, "pr_state")
        }
        async fn event_audit_rows(&self) -> anyhow::Result<Vec<AuditFact>> {
            answer_rows(&self.event_rows, "event_rows")
        }
        async fn decision_core_holds(&self, _: &str) -> anyhow::Result<Vec<AuditFact>> {
            answer_rows(&self.holds, "holds")
        }
    }

    fn branch(r: Result<bool, &'static str>) -> Stub {
        Stub {
            branch_task: Some(r),
            ..Stub::default()
        }
    }

    #[tokio::test]
    async fn mika2675_decide_a_suit_la_recherche_de_tache() {
        let c = || classify(GREEN_MAIN).unwrap();
        assert_eq!(
            decide(c(), &branch(Ok(false))).await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteDefaultBranch,
                target: "check_suite:senara-solutions/mika@main".into()
            }
        );
        assert_eq!(decide(c(), &branch(Ok(true))).await, Decision::Llm);
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(decide(c(), &branch(Err("db down"))).await, Decision::Llm);
    }

    #[tokio::test]
    async fn mika2675_decide_d_ne_consulte_aucun_etat() {
        let d = decide(classify(INERT).unwrap(), &Stub::default()).await;
        assert_eq!(
            d,
            Decision::Skip {
                class: PrefilterClass::InertLabel,
                target: "issue:senara-solutions/mika#2675".into()
            }
        );
    }

    /// AC4 : trois paliers, une coquille ne désarme pas.
    #[test]
    fn mika2675_interrupteur_trois_paliers() {
        use crate::qa_head_supersession::parse_switch;
        for (raw, armed) in [
            (None, true),
            (Some(""), true),
            (Some("1"), true),
            (Some("0"), false),
            (Some("false"), false),
            (Some("OFF"), false),
            (Some("of"), true),
        ] {
            assert_eq!(parse_switch(PREFILTER_ENV, raw), armed, "{raw:?}");
        }
    }

    /// Les valeurs d'audit sont un format de fil.
    #[test]
    fn mika2675_valeurs_daudit_format_de_fil() {
        assert_eq!(PREFILTER_SKIPPED_TOOL, "webhook_prefilter_skipped");
        assert_eq!(
            PrefilterClass::GreenCheckSuiteDefaultBranch.as_str(),
            "green_check_suite_default_branch"
        );
        assert_eq!(PrefilterClass::InertLabel.as_str(), "inert_label");
        assert_eq!(
            PrefilterClass::VerdictHoldTracked.as_str(),
            "verdict_hold_tracked"
        );
        assert_eq!(
            PrefilterClass::VerdictNonReviewer.as_str(),
            "verdict_non_reviewer"
        );
        assert_eq!(
            PrefilterClass::VerdictUnreadable.as_str(),
            "verdict_unreadable"
        );
        assert_eq!(
            PrefilterClass::VerdictPrClosed.as_str(),
            "verdict_pr_closed"
        );
        assert_eq!(
            PrefilterClass::GreenCheckSuiteHeadDecided.as_str(),
            "green_check_suite_head_decided"
        );
    }

    // ---- phase 2 — classe (c) -------------------------------------------

    /// Une revue telle que le gateway la délivre.
    pub(crate) fn review(state: &str, author: &str, body: &str) -> String {
        format!(
            "[GitHub] PR review ({state}) on senara-solutions/mika#2680 (fix(x): y) by @{author}\n\
             https://github.com/senara-solutions/mika/pull/2680#pullrequestreview-1\n\n{body}"
        )
    }

    const QA: &str = "mika-platform-qa";

    fn facts(text: &str) -> ReviewVerdictFacts {
        match classify(text) {
            Some(Candidate::ReviewVerdict(f)) => f,
            other => panic!("pas un verdict : {other:?}"),
        }
    }

    /// Terme par terme : chaque champ lu dans le texte est indépendant.
    #[test]
    fn mika2675_c_classify_lit_chaque_terme_separement() {
        let hold = facts(&review("commented", QA, "VERDICT: hold[review]"));
        assert!(hold.hold_tracked && !hold.non_reviewer_pass && !hold.unreadable);

        let other = facts(&review("approved", "samidarko", "VERDICT: pass"));
        assert!(!other.hold_tracked && other.non_reviewer_pass && !other.unreadable);

        let bad = facts(&review("commented", QA, "VERDICT: peut-être"));
        assert!(!bad.hold_tracked && !bad.non_reviewer_pass && bad.unreadable);

        let pass = facts(&review("approved", QA, "VERDICT: pass"));
        assert!(!pass.hold_tracked && !pass.non_reviewer_pass && !pass.unreadable);
        assert_eq!(
            pass.pr_url,
            "https://github.com/senara-solutions/mika/pull/2680"
        );

        assert!(facts(&review("commented", QA, "VERDICT: hold[Review]")).hold_tracked);
        // Seul le `pass` d'un non-relecteur est refermé par le handler : un
        // `block[*]` ou un `hold[x]` d'une autre identité garde son tour.
        for body in [
            "VERDICT: block[security]",
            "VERDICT: block[ac]",
            "VERDICT: hold[foo]",
        ] {
            assert!(
                !facts(&review("commented", "samidarko", body)).non_reviewer_pass,
                "{body}"
            );
        }
        // Un `hold[x]` inconnu n'est suivi par personne.
        assert!(!facts(&review("commented", QA, "VERDICT: hold[foo]")).hold_tracked);
        // Le suffixe `[bot]` reste le relecteur.
        assert!(
            !facts(&review(
                "approved",
                "mika-platform-qa[bot]",
                "VERDICT: pass"
            ))
            .non_reviewer_pass
        );
    }

    /// AC2 : une revue sans ligne VERDICT est une consigne, jamais filtrée.
    #[test]
    fn mika2675_c_revue_sans_ligne_verdict_hors_population() {
        for author in [QA, "samidarko"] {
            let text = review("commented", author, "Merci de renommer cette fonction.");
            assert_eq!(classify(&text), None, "{author}");
        }
    }

    fn pr(task: Result<bool, &'static str>) -> Stub {
        Stub {
            pr_task: Some(task),
            ..Stub::default()
        }
    }

    fn skip(class: PrefilterClass) -> Decision {
        Decision::Skip {
            class,
            target: "pr_review:senara-solutions/mika#2680".into(),
        }
    }

    /// Les trois termes lus dans le texte écartent sans consulter la forge.
    #[tokio::test]
    async fn mika2675_c_trois_termes_textuels_sans_forge() {
        for (body, author, class) in [
            (
                "VERDICT: hold[review]",
                QA,
                PrefilterClass::VerdictHoldTracked,
            ),
            (
                "VERDICT: pass",
                "samidarko",
                PrefilterClass::VerdictNonReviewer,
            ),
            ("VERDICT: peut-être", QA, PrefilterClass::VerdictUnreadable),
        ] {
            let c = classify(&review("commented", author, body)).unwrap();
            assert_eq!(
                decide(c, &pr(Ok(false))).await,
                skip(class),
                "{body} / {author}"
            );
        }
    }

    /// Le quatrième terme : PR fermée ou mergée côté forge.
    #[tokio::test]
    async fn mika2675_c_pr_fermee_ou_mergee() {
        for forge in [PrForgeState::Closed, PrForgeState::Merged] {
            let c = classify(&review("approved", QA, "VERDICT: pass")).unwrap();
            let state = Stub {
                pr_task: Some(Ok(false)),
                pr_state: Some(Ok(forge)),
                ..Stub::default()
            };
            assert_eq!(
                decide(c, &state).await,
                skip(PrefilterClass::VerdictPrClosed)
            );
        }
    }

    /// AC2 et AC3 : verdict actionnable, tâche active, forge ouverte ou
    /// illisible, base illisible — le tour a lieu.
    #[tokio::test]
    async fn mika2675_c_controles_negatifs() {
        let actionable = || classify(&review("approved", QA, "VERDICT: pass")).unwrap();
        let block = || classify(&review("changes_requested", QA, "VERDICT: block[ac]")).unwrap();
        for forge in [Ok(PrForgeState::Open), Err("gh 401")] {
            for c in [actionable(), block()] {
                let state = Stub {
                    pr_task: Some(Ok(false)),
                    pr_state: Some(forge),
                    ..Stub::default()
                };
                assert_eq!(decide(c, &state).await, Decision::Llm, "{forge:?}");
            }
        }
        // Tâche active : même un verdict non actionnable passe, sans forge.
        for (body, author) in [
            ("VERDICT: pass", QA),
            ("VERDICT: hold[review]", QA),
            ("VERDICT: pass", "samidarko"),
            ("VERDICT: peut-être", QA),
        ] {
            let c = classify(&review("approved", author, body)).unwrap();
            assert_eq!(decide(c, &pr(Ok(true))).await, Decision::Llm, "{body}");
        }
        // AC3 : base illisible.
        let c = classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap();
        assert_eq!(decide(c, &pr(Err("db down"))).await, Decision::Llm);
    }

    #[test]
    fn mika2675_c_etat_de_forge_lu_strictement() {
        assert_eq!(
            parse_pr_state(r#"{"state":"OPEN"}"#).unwrap(),
            PrForgeState::Open
        );
        assert_eq!(
            parse_pr_state("{\"state\":\"CLOSED\"}\n").unwrap(),
            PrForgeState::Closed
        );
        assert_eq!(
            parse_pr_state(r#"{"state":"MERGED"}"#).unwrap(),
            PrForgeState::Merged
        );
        for bad in [
            r#"{"state":"open"}"#,
            r#"{"state":null}"#,
            "{}",
            "",
            "gh: not found",
        ] {
            assert!(parse_pr_state(bad).is_err(), "{bad}");
        }
    }

    /// La règle « texte intact » : après le `verdict_handler` pour (c), par
    /// n'importe quel handler pour (a)/(d).
    #[test]
    fn mika2675_c_texte_intact_apres_le_verdict_handler() {
        let verdict = classify(&review("commented", QA, "VERDICT: hold[review]")).unwrap();
        let green = classify(GREEN_MAIN).unwrap();
        let by_verdict_only = Touched {
            by_any_handler: true,
            after_verdict_handler: false,
            outside_ci_success_handler: true,
        };
        let after = Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: true,
        };
        assert!(!by_verdict_only.applies_to(&verdict));
        assert!(after.applies_to(&verdict));
        assert!(by_verdict_only.applies_to(&green));
    }

    // ---- phase 3 — classe (b) -------------------------------------------

    const REPO: &str = "senara-solutions/mika";
    pub(crate) const HEAD_A: &str =
        "pr:senara-solutions/mika#2678@1a2edc8091fee6eec4c736b5dd974221802287b6";
    const HEAD_B: &str = "pr:senara-solutions/mika#2678@ffffffffffffffffffffffffffffffffffffffff";
    pub(crate) const GREEN_PR: &str =
        "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)";

    pub(crate) fn hold(target: &str, after: &str, trace: &str) -> AuditFact {
        AuditFact {
            // Rangs déterministes : « maintenant » / « now » écrit après tout le reste.
            id: if trace.contains("now") || trace.contains("maintenant") {
                10
            } else {
                1
            },
            tool: DECISION_CORE_HOLD_TOOL.into(),
            target: target.into(),
            after: Some(after.into()),
            trace: Some(trace.into()),
        }
    }

    fn processed(target: &str, trace: &str) -> AuditFact {
        AuditFact {
            id: 9,
            tool: "ci_success_handler_processed".into(),
            target: target.into(),
            after: None,
            trace: Some(trace.into()),
        }
    }

    /// Le cas mesuré (mika#2672, #2674) : cet événement redécide, un
    /// précédent avait décidé et notifié la même tête.
    fn decided_case() -> (Vec<AuditFact>, Vec<AuditFact>) {
        let mine = hold(HEAD_A, "notified", "rid-now");
        (
            vec![processed(HEAD_A, "rid-now"), mine.clone()],
            vec![mine, hold(HEAD_A, "notified", "rid-before")],
        )
    }

    #[test]
    fn mika2675_b_les_trois_termes_tenus_ecartent() {
        let (this_event, prior) = decided_case();
        assert_eq!(
            head_decided_and_notified(REPO, &this_event, &prior),
            Some(HEAD_A.to_string())
        );
    }

    /// Terme « décision attestée » : sans attestation de CET événement — dédup
    /// 2b/2c, pas de verdict, checks en attente, simple marqueur `processed` —
    /// le tour a lieu.
    #[test]
    fn mika2675_b_terme_decision_attestee() {
        let (_, prior) = decided_case();
        for this_event in [
            vec![],
            vec![processed(HEAD_A, "rid-now")],
            vec![AuditFact {
                tool: "ci_success_merge_ready".into(),
                ..processed(HEAD_A, "rid-now")
            }],
        ] {
            assert_eq!(
                head_decided_and_notified(REPO, &this_event, &prior),
                None,
                "{this_event:?}"
            );
        }
        // Une ligne antérieure d'un autre outil sur la même tête n'atteste pas
        // la décision.
        let (this_event, _) = decided_case();
        let prior = vec![AuditFact {
            tool: "ci_success_handler_human_gate_required".into(),
            ..hold(HEAD_A, "notified", "rid-before")
        }];
        assert_eq!(head_decided_and_notified(REPO, &this_event, &prior), None);
    }

    /// Terme « même tête » : une décision notifiée sur un AUTRE SHA de la même
    /// PR ne compte pas — une nouvelle tête n'est jamais filtrée (AC2).
    #[test]
    fn mika2675_b_terme_meme_tete() {
        let (this_event, _) = decided_case();
        let prior = vec![hold(HEAD_B, "notified", "rid-before")];
        assert_eq!(head_decided_and_notified(REPO, &this_event, &prior), None);
        // Et la tête de cet événement doit appartenir au dépôt de l'événement.
        let (this_event, prior) = decided_case();
        assert_eq!(
            head_decided_and_notified("senara-solutions/mika-cloud", &this_event, &prior),
            None
        );
    }

    /// Terme « notification attestée » : antérieure non notifiée, ou sans
    /// valeur, ne compte pas.
    #[test]
    fn mika2675_b_terme_notification_attestee() {
        let (this_event, _) = decided_case();
        for after in [Some("not_notified"), None] {
            let prior = vec![AuditFact {
                after: after.map(Into::into),
                ..hold(HEAD_A, "", "rid-before")
            }];
            assert_eq!(
                head_decided_and_notified(REPO, &this_event, &prior),
                None,
                "{after:?}"
            );
        }
    }

    /// Une attestation d'un autre événement écrite APRÈS celle de cet événement
    /// ne vaut pas « déjà » : le premier traitement d'une tête, lent, garde son
    /// tour même si un traitement plus tardif l'a devancé dans l'audit (AC2).
    #[test]
    fn mika2675_b_terme_anteriorite() {
        let (this_event, _) = decided_case();
        let later = AuditFact {
            id: 11,
            ..hold(HEAD_A, "notified", "rid-after")
        };
        assert_eq!(head_decided_and_notified(REPO, &this_event, &[later]), None);
    }

    /// Premier traitement d'une tête : la seule attestation est la sienne,
    /// même notifiée — elle ne vaut pas « déjà » (AC2).
    #[test]
    fn mika2675_b_premier_traitement_garde_son_tour() {
        let mine = hold(HEAD_A, "notified", "rid-now");
        assert_eq!(
            head_decided_and_notified(
                REPO,
                std::slice::from_ref(&mine),
                std::slice::from_ref(&mine)
            ),
            None
        );
    }

    fn rows(
        event: Result<Vec<AuditFact>, &'static str>,
        holds: Option<Result<Vec<AuditFact>, &'static str>>,
    ) -> Stub {
        Stub {
            event_rows: Some(event),
            holds,
            ..Stub::default()
        }
    }

    #[tokio::test]
    async fn mika2675_b_decide_ecarte_avec_cible_nommee() {
        let (this_event, prior) = decided_case();
        assert_eq!(
            decide(
                classify(GREEN_PR).unwrap(),
                &rows(Ok(this_event), Some(Ok(prior)))
            )
            .await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteHeadDecided,
                target: format!("check_suite:{}", HEAD_A.strip_prefix("pr:").unwrap()),
            }
        );
    }

    /// Sans attestation de cet événement, les attestations de la PR ne sont
    /// même pas lues (le bouchon paniquerait).
    #[tokio::test]
    async fn mika2675_b_decide_sans_attestation_ne_lit_pas_la_pr() {
        let c = classify(GREEN_PR).unwrap();
        assert_eq!(
            decide(c, &rows(Ok(vec![processed(HEAD_A, "rid-now")]), None)).await,
            Decision::Llm
        );
    }

    /// AC3 : audit illisible, à l'une ou l'autre lecture ⇒ le tour a lieu.
    #[tokio::test]
    async fn mika2675_b_audit_illisible_le_tour_a_lieu() {
        let (this_event, _) = decided_case();
        let c = || classify(GREEN_PR).unwrap();
        assert_eq!(
            decide(c(), &rows(Err("db down"), None)).await,
            Decision::Llm
        );
        assert_eq!(
            decide(c(), &rows(Ok(this_event), Some(Err("db down")))).await,
            Decision::Llm
        );
    }

    #[test]
    fn mika2675_b_prefixe_de_pr() {
        assert_eq!(pr_prefix(HEAD_A), Some("pr:senara-solutions/mika#2678@"));
        assert_eq!(pr_prefix("pr:senara-solutions/mika#2678"), None);
    }

    /// (b) n'admet qu'un texte touché par `ci_success_handler` seul.
    #[test]
    fn mika2675_b_texte_touche_hors_ci_success_handler() {
        let b = classify(GREEN_PR).unwrap();
        let by = |outside| Touched {
            by_any_handler: true,
            after_verdict_handler: true,
            outside_ci_success_handler: outside,
        };
        assert!(!by(false).applies_to(&b));
        assert!(by(true).applies_to(&b));
    }
}
