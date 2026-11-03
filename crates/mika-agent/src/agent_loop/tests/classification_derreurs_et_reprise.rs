//! Classification des erreurs d'outil, revue de PR et intention de reprise.
//!
//! Entrée informationnelle et sortie persistable (#648), erreurs terminales
//! et échec terminal d'outil requis, succès de revue de PR et la porte qu'il
//! lève (#695, #821), `detect_resume_intent`, la portée d'intention
//! (mika#3002) et `resume_reconcile_satisfied`.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- Persistence evaluation guard detection tests (#648) --

#[test]
fn test_detect_informational_input_fyi() {
    let result = detect_informational_input("FYI the deploy succeeded");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "FYI");
}

#[test]
fn test_detect_informational_input_heads_up() {
    let result = detect_informational_input("Heads up — the CI pipeline is red");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "Heads up");
}

#[test]
fn test_detect_informational_input_diagnostic() {
    let result = detect_informational_input("Running a diagnostic on the memory system");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "diagnostic");
}

#[test]
fn test_detect_informational_input_correction() {
    let result = detect_informational_input("actually, the timeout is 30s not 60s");
    assert!(result.is_some());
    // trim_start() ensures no leading whitespace from the (?:^|\s) anchor
    assert_eq!(result.unwrap(), "actually,");
}

#[test]
fn test_detect_informational_input_no_match() {
    assert!(detect_informational_input("Can you fix the bug in the parser?").is_none());
}

#[test]
fn test_detect_informational_input_empty() {
    assert!(detect_informational_input("").is_none());
}

#[test]
fn test_detect_persistable_output_root_cause() {
    let result =
        detect_persistable_output("The root cause was a connection timeout in the retry loop");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "root cause");
}

#[test]
fn test_detect_persistable_output_confirms() {
    let result =
        detect_persistable_output("This confirms the diagnosis — the issue was in the parser");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "This confirms");
}

#[test]
fn test_detect_persistable_output_validated() {
    let result = detect_persistable_output("I validated that the fix works correctly");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "validated that");
}

#[test]
fn test_detect_persistable_output_lesson_learned() {
    let result = detect_persistable_output("Key lesson learned: always check the return code");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "lesson learned");
}

#[test]
fn test_detect_persistable_output_no_match() {
    assert!(detect_persistable_output("Here's the code change you requested").is_none());
}

#[test]
fn test_detect_persistable_output_future_tense_no_match() {
    // "I'll verify" is future-tense — should not trigger
    assert!(detect_persistable_output("I'll verify the fix later").is_none());
}

#[test]
fn test_detect_persistable_output_empty() {
    assert!(detect_persistable_output("").is_none());
}

#[test]
fn test_detect_informational_input_incident_report() {
    let result = detect_informational_input("incident report: prod database was down for 5 min");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "incident report");
}

// -- is_terminal_tool_error tests --

#[test]
fn test_terminal_error_github_self_approval() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nGraphQL: Can not approve your own pull request"
    ));
}

#[test]
fn test_terminal_error_github_self_review() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nYou can't review your own pull request"
    ));
}

#[test]
fn test_terminal_error_http_404() {
    assert!(is_terminal_tool_error("HTTP 404: Not Found"));
}

#[test]
fn test_terminal_error_http_403() {
    assert!(is_terminal_tool_error("HTTP 403: Forbidden"));
}

#[test]
fn test_terminal_error_http_401() {
    assert!(is_terminal_tool_error("HTTP 401: Unauthorized"));
}

#[test]
fn test_terminal_error_insufficient_permissions() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\ninsufficient permissions for this resource"
    ));
}

#[test]
fn test_terminal_error_resource_not_accessible() {
    assert!(is_terminal_tool_error(
        "Resource not accessible by integration"
    ));
}

#[test]
fn test_terminal_error_permission_denied() {
    assert!(is_terminal_tool_error("permission denied"));
}

#[test]
fn test_terminal_error_case_insensitive() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nGraphQL: CAN NOT APPROVE YOUR OWN pull request"
    ));
}

#[test]
fn test_retryable_error_http_429() {
    assert!(!is_terminal_tool_error("HTTP 429: rate limit exceeded"));
}

#[test]
fn test_retryable_error_rate_limit() {
    assert!(!is_terminal_tool_error(
        "API rate limit exceeded. Please retry after 60 seconds."
    ));
}

#[test]
fn test_retryable_error_http_500() {
    assert!(!is_terminal_tool_error("HTTP 500: Internal Server Error"));
}

#[test]
fn test_retryable_error_http_502() {
    assert!(!is_terminal_tool_error("HTTP 502: Bad Gateway"));
}

#[test]
fn test_retryable_error_http_503() {
    assert!(!is_terminal_tool_error("HTTP 503: Service Unavailable"));
}

#[test]
fn test_retryable_error_http_504() {
    assert!(!is_terminal_tool_error("HTTP 504: Gateway Timeout"));
}

#[test]
fn test_retryable_error_timeout() {
    assert!(!is_terminal_tool_error("request timed out after 30s"));
}

#[test]
fn test_retryable_error_connection_refused() {
    assert!(!is_terminal_tool_error("connection refused"));
}

#[test]
fn test_retryable_overrides_terminal() {
    // A 429 with "permission denied" text should still be retryable
    assert!(!is_terminal_tool_error(
        "HTTP 429: permission denied rate limit exceeded"
    ));
}

#[test]
fn test_not_found_bare_is_not_terminal() {
    // Bare "not found" should NOT be terminal — too broad, matches search results
    assert!(!is_terminal_tool_error("No matching records found"));
    assert!(!is_terminal_tool_error("file not found: /tmp/data"));
}

#[test]
fn test_forbidden_bare_is_not_terminal() {
    // Bare "forbidden" should NOT be terminal — too broad
    assert!(!is_terminal_tool_error(
        "some resources are forbidden by default"
    ));
}

#[test]
fn test_unauthorized_bare_is_not_terminal() {
    // Bare "unauthorized" should NOT be terminal — too broad
    assert!(!is_terminal_tool_error(
        "unauthorized access attempt logged"
    ));
}

#[test]
fn test_unknown_error_not_terminal() {
    assert!(!is_terminal_tool_error("some random error message"));
}

#[test]
fn test_empty_output_not_terminal() {
    assert!(!is_terminal_tool_error(""));
}

#[test]
fn test_successful_output_not_terminal() {
    assert!(!is_terminal_tool_error("PR approved successfully"));
}

// -- has_terminal_required_tool_failure tests --

// `make_summary` est promu dans `tests/mod.rs` : les gardes de tour webhook le
// lisent aussi.

#[test]
fn test_terminal_required_tool_failure_detected() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "Exit code: 1\nGraphQL: Can not approve your own pull request",
        false,
    )];
    assert!(has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_non_required_tool_ignored() {
    let required: HashSet<String> = ["qa_pr_view"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "Exit code: 1\nGraphQL: Can not approve your own pull request",
        false,
    )];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_successful_tool_ignored() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "PR approved successfully", true)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_retryable_error_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "HTTP 429: rate limit exceeded",
        false,
    )];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_unknown_error_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "some random error", false)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_empty_summaries() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    assert!(!has_terminal_required_tool_failure(&required, &[]));
}

#[test]
fn test_terminal_failure_empty_output_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "", false)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_multiple_summaries_one_terminal() {
    let required: HashSet<String> = ["run_gh", "qa_pr_view"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let summaries = vec![
        make_summary("qa_pr_view", "PR #42: Fix bug", true),
        make_summary("run_gh", "Exit code: 1\nHTTP 404: Not Found", false),
    ];
    assert!(has_terminal_required_tool_failure(&required, &summaries));
}

// -- has_successful_pr_review tests (#695) --

#[test]
fn pr_review_detected_when_run_gh_pr_review_succeeded() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","455","--approve","--body","VERDICT: pass"]}"#
            .to_string(),
        output_summary: "Review submitted".to_string(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_for_failed_review() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","455","--approve"]}"#.to_string(),
        output_summary: "Exit code: 1\nHTTP 422: Unprocessable Entity".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(!has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_for_pr_list() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","list","--state","open"]}"#.to_string(),
        output_summary: "PR #1 Fix bug".to_string(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_when_empty() {
    assert!(!has_successful_pr_review(&[]));
}

// -- required-tools gate skipped after PR review success (#821, Fix B) --

#[test]
fn required_tools_gate_skipped_after_pr_review_success() {
    // Simulate: successful `run_gh pr review` in tool summaries.
    // Required tools include a tool not called (e.g., qa_pr_view).
    // has_successful_pr_review should return true, which means the
    // required-tools gate should skip instead of re-prompting.
    let summaries = vec![
            ToolCallSummary {
                step: 1,
                name: "run_gh".to_string(),
                input_summary:
                    r#"{"command":["pr","review","https://github.com/org/repo/pull/42","--approve","--body","VERDICT: pass"]}"#
                        .to_string(),
                output_summary: "Review submitted".to_string(),
                success: true,
                non_zero_exit: false,
            },
        ];

    // This is the condition checked in the required-tools gate (Fix B).
    // When true, the gate skips re-prompting.
    assert!(
        has_successful_pr_review(&summaries),
        "should detect successful PR review in tool summaries"
    );

    // Verify it returns false for the negative case (failed review).
    let failed_summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","42","--approve"]}"#.to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(
        !has_successful_pr_review(&failed_summaries),
        "failed review should not trigger early-accept"
    );
}

// -- detect_resume_intent tests --

#[test]
fn resume_intent_detected_for_milestone() {
    assert!(detect_resume_intent("resume mika milestone#8"));
    assert!(detect_resume_intent("Resume mika milestone#8"));
    assert!(detect_resume_intent("please resume mika milestone #8"));
}

#[test]
fn resume_intent_detected_for_project() {
    assert!(detect_resume_intent("continue mika project#3"));
    assert!(detect_resume_intent("Continue mika project #3"));
}

#[test]
fn resume_intent_not_detected_without_process_ref() {
    assert!(!detect_resume_intent("resume the task"));
    assert!(!detect_resume_intent("continue working on the feature"));
    assert!(!detect_resume_intent("resume"));
}

#[test]
fn resume_intent_not_detected_without_verb() {
    assert!(!detect_resume_intent("check mika milestone#8"));
    assert!(!detect_resume_intent("milestone#8 status"));
}

#[test]
fn resume_intent_not_detected_on_regular_messages() {
    assert!(!detect_resume_intent("How's the project going?"));
    assert!(!detect_resume_intent("Can you explain the architecture?"));
    assert!(!detect_resume_intent(""));
}

// -- IntentScope tests (mika#3002) --

/// Un plan de groom ordinaire : il parle de reprise parce que c'est son sujet, et il
/// cite un jalon. Les deux conjoints de `detect_resume_intent` sont là, et aucune
/// intention de reprise n'est adressée à l'agent.
///
/// Les deux lignes qui portent les conjoints sont celles **mesurées** sur le plan de
/// mika#2995 (`tests/fixtures/review_anchor_unparsed_retry_2996/plan-mika2995-61a3fb572.md`,
/// l. 479 et 506), le brief de l'incident `3a066c59` : le verbe vient du français « le
/// prompt servi *continue* de prescrire », que `\b(resume|continue)\b` attrape mot pour
/// mot, et le processus d'un « milestone #33 » **cité**. « reprendre le vol suivant »,
/// que nomme AC2, est là aussi mais ne porte aucun des deux — c'est le vocabulaire du
/// défaut, pas son déclencheur.
const PLAN_DE_GROOM: &str = "## 4. Ordre de vol\n\nLe premier vol livre le détecteur ; \
         il faudra reprendre le vol suivant une fois la sonde relevée.\n\nLe défaut survit \
         au correctif : le prompt servi continue de prescrire l'ancienne forme.\n\nLié : \
         milestone #33, dont ce ticket est l'enfant.\n";

/// Le plan sous l'enveloppe de mika#2996, précédé d'une vraie instruction à l'agent.
fn brief_enveloppe(tete: &str) -> String {
    format!(
        "{tete}{}\n{PLAN_DE_GROOM}{}\n",
        review_anchor::RETRY_PLAN_BEGIN,
        review_anchor::RETRY_PLAN_END
    )
}

/// Le contrôle qui fait tenir tous les autres : ce plan déclenche bien la garde quand
/// le message entier est lu. Sans lui, les tests de stand-down passeraient aussi sur un
/// plan que le détecteur n'a jamais vu.
#[test]
fn mika3002_le_plan_de_groom_declenche_sur_le_message_entier() {
    assert!(
        IntentScope::WholeMessage.triggered(detect_resume_intent, PLAN_DE_GROOM, true),
        "la fixture ne porte plus les deux conjoints : les cas de stand-down ne \
             mesureraient rien"
    );
}

/// AC1 — sur un tour de revue, un brief sans enveloppe EST le plan : rien n'y est
/// adressé à l'agent, donc la garde ne s'arme pas. C'est la forme de l'incident
/// `3a066c59`.
#[test]
fn mika3002_sur_un_tour_de_revue_le_plan_seul_ne_declenche_pas() {
    assert!(!IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, PLAN_DE_GROOM, true));
}

/// Hors tour de revue, le rétrécissement ne s'applique pas : même entrée, même
/// message, comportement d'avant mika#3002. C'est ce qui borne le stand-down à sa
/// population.
#[test]
fn mika3002_hors_tour_de_revue_le_comportement_est_inchange() {
    assert!(IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, PLAN_DE_GROOM, false));
}

/// AC2 — une vraie instruction de reprise adressée à l'agent vit HORS du plan, donc
/// dans la région lue : la garde s'arme toujours.
#[test]
fn mika3002_une_instruction_hors_du_plan_declenche_toujours() {
    let brief = brief_enveloppe("resume mika milestone#8\n\n");
    assert!(IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, &brief, true));
}

/// `stood_down` ne dit vrai que quand le rétrécissement a réellement changé l'issue —
/// c'est le contrat de la sonde de contrôle positif, pas un synonyme de « tour de
/// revue ».
#[test]
fn mika3002_stood_down_ne_vaut_que_quand_le_retrecissement_change_lissue() {
    // Le cas qu'AC3 compte : le message entier déclenchait, la région lue non.
    assert!(IntentScope::OutsideReviewedPlan.stood_down(detect_resume_intent, PLAN_DE_GROOM, true));
    // Une instruction hors plan : la garde s'arme, donc aucun stand-down.
    let brief = brief_enveloppe("resume mika milestone#8\n\n");
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(detect_resume_intent, &brief, true));
    // Hors tour de revue : rien n'a été rétréci.
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(
        detect_resume_intent,
        PLAN_DE_GROOM,
        false
    ));
    // Un message sans intention de reprise : il n'y avait rien à désarmer.
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(
        detect_resume_intent,
        "Can you explain the architecture?",
        true
    ));
    // Une entrée à portée entière ne se désarme jamais, même sur un tour de revue.
    assert!(!IntentScope::WholeMessage.stood_down(detect_resume_intent, PLAN_DE_GROOM, true));
}

/// `resume_reconcile` est la seule entrée rétrécie. Élargir le stand-down au reste du
/// registre serait un changement non mesuré déguisé en cohérence : aucune autre entrée
/// n'a de prémisse fausse sur un tour de revue, toutes exigent un préfixe `[GitHub]` ou
/// `[callback: …]` qu'un brief de plan ne porte pas.
#[test]
fn mika3002_resume_reconcile_est_la_seule_entree_retrecie() {
    let retrecies: Vec<&str> = INTENT_GUARDS
        .iter()
        .filter(|g| matches!(g.scope, IntentScope::OutsideReviewedPlan))
        .map(|g| g.label)
        .collect();
    assert_eq!(retrecies, vec!["resume_reconcile"]);
}

// -- resume_reconcile_satisfied tests --

#[test]
fn resume_satisfied_with_successful_list_tasks() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "list_tasks".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_satisfied_with_successful_check_task() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "check_task".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_with_failed_list_tasks() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "list_tasks".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: false,
    }];
    assert!(!resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_with_unrelated_tool() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "send_message".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_when_empty() {
    assert!(!resume_reconcile_satisfied(&[]));
}
