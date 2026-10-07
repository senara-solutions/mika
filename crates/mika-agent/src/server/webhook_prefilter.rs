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
//! Les classes (b) et (c) du ticket viennent en phases 2 et 3 ; elles ajoutent
//! une variante à [`PrefilterClass`] et ses faits, pas un second mécanisme.

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;

use super::ci_success_handler::parse_check_suite_success;
use super::webhook_queue_v2::{WebhookEventKind, classify_event};

/// Kill-switch, armé par défaut. Même table de vérité que les interrupteurs
/// de mika#2671 ([`crate::qa_head_supersession::parse_switch`]).
pub(crate) const PREFILTER_ENV: &str = "MIKA_WEBHOOK_PREFILTER";

/// `audit_events.tool_name` d'un événement écarté. **SOLE WRITER** :
/// [`skip_turn`].
pub(crate) const PREFILTER_SKIPPED_TOOL: &str = "webhook_prefilter_skipped";

/// La branche par défaut des dépôts que la boucle sert.
pub(crate) const DEFAULT_BRANCH: &str = "main";

/// Une classe d'événement écartée sans LLM. Valeur de `after_value` dans
/// l'audit : un format de fil, épinglé par test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefilterClass {
    /// (a) `check_suite` verte sur la branche par défaut, sans tâche active.
    GreenCheckSuiteDefaultBranch,
    /// (d) label qu'aucune partie de la boucle ne lit.
    InertLabel,
}

impl PrefilterClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::GreenCheckSuiteDefaultBranch => "green_check_suite_default_branch",
            Self::InertLabel => "inert_label",
        }
    }
}

/// Un événement qui POURRAIT être écarté, avant consultation de l'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Candidate {
    GreenDefaultBranch {
        repo: String,
        branch: String,
    },
    InertLabel {
        repo: String,
        issue: u64,
        label: String,
    },
}

/// Fonction pure : classe le texte du gateway. `None` = hors population, le
/// tour a lieu. Lit les grammaires existantes, n'en écrit aucune.
pub(crate) fn classify(text: &str) -> Option<Candidate> {
    if let Some(event) = parse_check_suite_success(text) {
        return (event.branch == DEFAULT_BRANCH).then_some(Candidate::GreenDefaultBranch {
            repo: event.repo,
            branch: event.branch,
        });
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

/// L'issue de la décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Llm,
    Skip {
        class: PrefilterClass,
        target: String,
    },
}

/// Décide un candidat. `branch_has_active_task` est la seule recherche d'état
/// de la phase 1, injectée pour que les tests la bouchonnent ; elle n'est
/// appelée que pour (a). Une erreur de recherche laisse passer (AC3).
pub(crate) async fn decide<F, Fut>(candidate: Candidate, branch_has_active_task: F) -> Decision
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<bool>>,
{
    match candidate {
        Candidate::GreenDefaultBranch { repo, branch } => {
            match branch_has_active_task(branch.clone()).await {
                Ok(false) => Decision::Skip {
                    class: PrefilterClass::GreenCheckSuiteDefaultBranch,
                    target: format!("check_suite:{repo}@{branch}"),
                },
                Ok(true) => Decision::Llm,
                Err(e) => {
                    warn!(
                        event = "webhook_prefilter_state_unreadable",
                        repo = %repo,
                        branch = %branch,
                        error = %e,
                        "pré-filtre : état illisible — le tour a lieu (fail-safe)"
                    );
                    Decision::Llm
                }
            }
        }
        Candidate::InertLabel { repo, issue, .. } => Decision::Skip {
            class: PrefilterClass::InertLabel,
            target: format!("issue:{repo}#{issue}"),
        },
    }
}

/// L'interrupteur est-il armé ? Lu à chaque événement.
pub(crate) fn prefilter_enabled() -> bool {
    crate::qa_head_supersession::parse_switch(
        PREFILTER_ENV,
        std::env::var(PREFILTER_ENV).ok().as_deref(),
    )
}

/// La porte. Rend `true` quand le tour LLM est écarté ; une ligne `info!` et
/// une ligne d'audit nommées sont alors écrites. `handlers_touched` dit si un
/// handler a remplacé ou enrichi le texte.
pub(crate) async fn skip_turn(
    db: &AsyncDatabase,
    original_text: &str,
    handlers_touched: bool,
    request_id: &str,
    enabled: bool,
) -> bool {
    if !enabled || handlers_touched {
        return false;
    }
    let Some(candidate) = classify(original_text) else {
        return false;
    };
    let Decision::Skip { class, target } = decide(candidate, |branch| async move {
        Ok(db.find_active_task_by_branch(&branch).await?.is_some())
    })
    .await
    else {
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
mod tests {
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

    /// (a) terme par terme : conclusion, branche, préfixe.
    #[test]
    fn mika2675_classify_a_chaque_terme_sort_de_la_population() {
        for text in [
            "[GitHub] Check suite failure on senara-solutions/mika (branch: main)",
            "[GitHub] Check suite timed_out on senara-solutions/mika (branch: main)",
            "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2675/x)",
            "[GitHub] Check suite success on senara-solutions/mika (branch: mainline)",
            "Check suite success on senara-solutions/mika (branch: main)",
        ] {
            assert_eq!(classify(text), None, "{text}");
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

    #[tokio::test]
    async fn mika2675_decide_a_suit_la_recherche_de_tache() {
        let c = || classify(GREEN_MAIN).unwrap();
        assert_eq!(
            decide(c(), |_| async { Ok(false) }).await,
            Decision::Skip {
                class: PrefilterClass::GreenCheckSuiteDefaultBranch,
                target: "check_suite:senara-solutions/mika@main".into()
            }
        );
        assert_eq!(decide(c(), |_| async { Ok(true) }).await, Decision::Llm);
        // AC3 : base illisible ⇒ le tour a lieu.
        assert_eq!(
            decide(c(), |_| async { anyhow::bail!("db down") }).await,
            Decision::Llm
        );
    }

    #[tokio::test]
    async fn mika2675_decide_d_ne_consulte_aucun_etat() {
        let d = decide(classify(INERT).unwrap(), |_| async {
            panic!("aucune recherche d'état pour un label inerte")
        })
        .await;
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
    }
}
