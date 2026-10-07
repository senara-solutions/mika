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
//! La classe (b) vient en phase 3 ; elle ajoutera une variante et ses faits,
//! pas un second mécanisme.

use tracing::{info, warn};

use crate::async_db::AsyncDatabase;

use super::ci_success_handler::parse_check_suite_success;
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
    /// (c) verdict d'une identité qui n'est pas le relecteur QA.
    VerdictNonReviewer,
    /// (c) ligne `VERDICT:` présente mais illisible.
    VerdictUnreadable,
    /// (c) PR fermée ou mergée côté forge.
    VerdictPrClosed,
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
    /// L'auteur n'est pas `REVIEWER_FORGE_LOGIN`.
    pub(crate) non_reviewer: bool,
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
        return (event.branch == DEFAULT_BRANCH).then_some(Candidate::GreenDefaultBranch {
            repo: event.repo,
            branch: event.branch,
        });
    }
    if let Some(event) = parse_pr_review_event(text) {
        // Pas de ligne VERDICT : une revue-consigne, jamais filtrée (AC2).
        verdict_raw_value(&event.body)?;
        let verdict = parse_verdict(&event.body);
        let pr_url = event.pr_url();
        return Some(Candidate::ReviewVerdict(ReviewVerdictFacts {
            hold_tracked: matches!(&verdict, Verdict::Hold(r) if r.eq_ignore_ascii_case("review")),
            non_reviewer: !mika_common::forge_identity::is_reviewer_forge_login(&event.reviewer),
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

/// Les recherches d'état du pré-filtre, injectées pour que les tests les
/// bouchonnent : aucun appel GitHub réel en test.
pub(crate) trait PrefilterState {
    async fn branch_has_active_task(&self, branch: &str) -> anyhow::Result<bool>;
    async fn pr_has_active_task(&self, pr_url: &str) -> anyhow::Result<bool>;
    async fn pr_state(&self, repo: &str, pr_number: u64) -> anyhow::Result<PrForgeState>;
}

/// L'état réel : base de l'agent, et `gh` pour la forge.
pub(crate) struct LiveState<'a> {
    pub(crate) db: &'a AsyncDatabase,
    pub(crate) github_token: Option<&'a str>,
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
        Candidate::InertLabel { repo, issue, .. } => Decision::Skip {
            class: PrefilterClass::InertLabel,
            target: format!("issue:{repo}#{issue}"),
        },
        Candidate::ReviewVerdict(facts) => decide_review_verdict(facts, state).await,
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
    } else if facts.non_reviewer {
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
/// intact ; (c) exige un texte intact APRÈS le `verdict_handler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Touched {
    pub(crate) by_any_handler: bool,
    pub(crate) after_verdict_handler: bool,
}

impl Touched {
    fn applies_to(self, candidate: &Candidate) -> bool {
        match candidate {
            Candidate::ReviewVerdict(_) => self.after_verdict_handler,
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

    /// État bouchonné. `None` = la recherche ne doit pas être consultée :
    /// elle panique si elle l'est. Aucun appel GitHub réel.
    #[derive(Default)]
    pub(crate) struct Stub {
        pub(crate) branch_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_task: Option<Result<bool, &'static str>>,
        pub(crate) pr_state: Option<Result<PrForgeState, &'static str>>,
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
        assert!(hold.hold_tracked && !hold.non_reviewer && !hold.unreadable);

        let other = facts(&review("approved", "samidarko", "VERDICT: pass"));
        assert!(!other.hold_tracked && other.non_reviewer && !other.unreadable);

        let bad = facts(&review("commented", QA, "VERDICT: peut-être"));
        assert!(!bad.hold_tracked && !bad.non_reviewer && bad.unreadable);

        let pass = facts(&review("approved", QA, "VERDICT: pass"));
        assert!(!pass.hold_tracked && !pass.non_reviewer && !pass.unreadable);
        assert_eq!(
            pass.pr_url,
            "https://github.com/senara-solutions/mika/pull/2680"
        );

        // Un `hold[x]` inconnu n'est suivi par personne.
        assert!(!facts(&review("commented", QA, "VERDICT: hold[foo]")).hold_tracked);
        // Le suffixe `[bot]` reste le relecteur.
        assert!(
            !facts(&review(
                "approved",
                "mika-platform-qa[bot]",
                "VERDICT: pass"
            ))
            .non_reviewer
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
            (
                "VERDICT: block[ac]",
                "mika-platform-dev",
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
        };
        let after = Touched {
            by_any_handler: true,
            after_verdict_handler: true,
        };
        assert!(!by_verdict_only.applies_to(&verdict));
        assert!(after.applies_to(&verdict));
        assert!(by_verdict_only.applies_to(&green));
    }
}
