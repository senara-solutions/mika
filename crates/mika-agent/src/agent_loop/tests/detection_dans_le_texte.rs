//! La détection d'intentions dans le texte que le modèle rend.
//!
//! Appels d'outil en balise ou en prose (#569), revendication d'achèvement
//! (#483), clôture de jalon sans patch, extraction du numéro et du jeu de
//! PATCH, et l'`argv` de `run_gh` qui la porte (#797, #1182, #1207).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- detect_text_based_tool_call tests --

#[test]
fn test_detect_text_based_tool_call_function_tag() {
    assert!(detect_text_based_tool_call(
        "<function=search_memory>{\"query\":\"test\"}</function>"
    ));
}

#[test]
fn test_detect_text_based_tool_call_tool_call_wrapper() {
    assert!(detect_text_based_tool_call(
        "<tool_call><function=search>{}</function></tool_call>"
    ));
}

#[test]
fn test_detect_text_based_tool_call_plain_text() {
    assert!(!detect_text_based_tool_call(
        "I found some information about your meetings."
    ));
}

#[test]
fn test_detect_text_based_tool_call_empty() {
    assert!(!detect_text_based_tool_call(""));
}

#[test]
fn test_detect_text_based_tool_call_partial_tag() {
    // Only opening tag without closing — should NOT trigger.
    assert!(!detect_text_based_tool_call(
        "Use <function=search_memory to find things"
    ));
}

// -- detect_prose_style_tool_call tests (#569) --

/// Helper: build a tool name set from a slice of names.
fn tool_set(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_detect_prose_tool_call_known_tool() {
    let tools = tool_set(&["check_work_item"]);
    assert_eq!(
        detect_prose_style_tool_call(
            r#"check_work_item({"task_id": "48cbb025-6d8e-430f-a957-9ce2e32800bb"})"#,
            &tools,
        ),
        Some("check_work_item".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_whitespace_between_parens_and_brace() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"search_memory( {"query": "test"} )"#, &tools,),
        Some("search_memory".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_multiline_json() {
    let tools = tool_set(&["store_fact"]);
    let text = "store_fact(\n{\"key\": \"val\",\n\"key2\": \"val2\"}\n)";
    assert_eq!(
        detect_prose_style_tool_call(text, &tools),
        Some("store_fact".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_unknown_identifier() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"my_function({"key": "val"})"#, &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_empty_text() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(detect_prose_style_tool_call("", &tools), None);
}

#[test]
fn test_detect_prose_tool_call_parens_but_no_tool_pattern() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call("I found some information (see details).", &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_tool_name_without_invocation() {
    let tools = tool_set(&["check_work_item"]);
    assert_eq!(
        detect_prose_style_tool_call("Use check_work_item to verify the task status.", &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_generic_function_not_in_toolset() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"Run my_func({"x": 1}) to test"#, &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_returns_first_match() {
    let tools = tool_set(&["search_memory", "store_fact"]);
    let text = r#"search_memory({"query": "a"}) and store_fact({"text": "b"})"#;
    assert_eq!(
        detect_prose_style_tool_call(text, &tools),
        Some("search_memory".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_underscores_and_digits() {
    let tools = tool_set(&["tool_v2"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"tool_v2({"a": 1})"#, &tools),
        Some("tool_v2".to_string()),
    );
}

// -- detect_completion_claim tests (#483) --

#[test]
fn test_detect_completion_claim_merged() {
    assert_eq!(
        detect_completion_claim("PR merged successfully"),
        Some("merged")
    );
}

#[test]
fn test_detect_completion_claim_completed() {
    assert_eq!(detect_completion_claim("Task completed"), Some("completed"));
}

#[test]
fn test_detect_completion_claim_complete() {
    assert_eq!(
        detect_completion_claim("The migration is complete"),
        Some("complete")
    );
}

#[test]
fn test_detect_completion_claim_deployed() {
    assert_eq!(
        detect_completion_claim("Successfully deployed to production"),
        Some("deployed")
    );
}

#[test]
fn test_detect_completion_claim_shipped() {
    assert_eq!(
        detect_completion_claim("Feature shipped in v2.1"),
        Some("shipped")
    );
}

#[test]
fn test_detect_completion_claim_case_insensitive() {
    assert_eq!(detect_completion_claim("MERGED the PR"), Some("MERGED"));
    assert_eq!(
        detect_completion_claim("Successfully Deployed"),
        Some("Deployed")
    );
}

#[test]
fn test_detect_completion_claim_no_match_on_done() {
    // "done" intentionally excluded — too many false positives
    assert!(detect_completion_claim("I'm done analyzing").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_built() {
    // "built" intentionally excluded — too many false positives
    assert!(detect_completion_claim("I built a query for you").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_finished() {
    assert!(detect_completion_claim("I finished the analysis").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_plain_text() {
    assert!(detect_completion_claim("Here is the analysis result").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_substring() {
    // "unmerged" should NOT match due to word boundary
    assert!(detect_completion_claim("the unmerged changes").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_empty() {
    assert!(detect_completion_claim("").is_none());
}

#[test]
fn test_detect_completion_claim_word_boundary_redeployed() {
    // "redeployed" contains "deployed" but word boundary should still match
    // because "redeployed" has "deployed" at a word boundary after the prefix
    // Actually regex \b matches between "re" and "deployed"? No — \bdeployed\b
    // requires a word boundary before "deployed". In "redeployed", there's no
    // boundary between "re" and "deployed", so it should NOT match.
    assert!(detect_completion_claim("the service was redeployed").is_none());
}

#[test]
fn test_detect_completion_claim_in_sentence() {
    assert_eq!(
        detect_completion_claim("I've merged the PR and synced main. Everything looks good."),
        Some("merged")
    );
}

// -- detect_milestone_close_claim_without_patch tests (#797, #1207) --

/// Helper: build a ToolCallSummary for run_gh with the given input.
fn run_gh_summary(input: &str) -> ToolCallSummary {
    ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: input.to_string(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }
}

#[test]
fn test_detect_milestone_close_claim_with_patch_passes() {
    // First-person claim AND matching PATCH → returns None (no violation).
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries,)
            .is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_without_patch_caught() {
    // First-person claim AND no PATCH argv → returns the matched keyword.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#17, tasks reconciled, memory updated",
        &summaries,
    );
    assert!(result.is_some());
    assert!(result.unwrap().contains("closed"));
}

#[test]
fn test_detect_milestone_close_claim_case_insensitive() {
    // "I CLOSED MILESTONE" caught regardless of case.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I CLOSED MILESTONE #17 successfully",
        &summaries,
    );
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_no_match_on_unrelated_close() {
    // "PR closed" alone does NOT trigger — no first-person + milestone.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch("PR closed successfully", &summaries).is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_readback_alone_not_sufficient() {
    // Only readback argv (api ... --jq .state), no PATCH → still triggers.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","/repos/senara-solutions/mika/milestones/17","--jq",".state"]}"#,
    )];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_no_match_on_empty_text() {
    let summaries = vec![];
    assert!(detect_milestone_close_claim_without_patch("", &summaries).is_none());
}

#[test]
fn test_detect_milestone_close_claim_third_person_no_longer_matches() {
    // #1207: third-person planning prose no longer triggers the guard.
    // This replaces the old `_future_tense_overmatches_intentionally` test.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch(
            "The milestone is ready to close now",
            &summaries,
        )
        .is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_planning_prose_no_match() {
    // #1207: the incident text — mika-arch reviewing a brief about
    // milestone workflows. Third-person planning shape must not fire.
    let summaries = vec![];
    assert!(detect_milestone_close_claim_without_patch(
            "the plan proposes mika-dev call gh api PATCH /repos/owner/repo/milestones/789 to close the milestone",
            &summaries,
        )
        .is_none());
}

#[test]
fn test_detect_milestone_close_claim_planning_prose_no_number_no_match() {
    // #1207 C3b: third-person without API path or number.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch(
            "the plan proposes mika-dev close the milestone after merge",
            &summaries,
        )
        .is_none()
    );
}

#[test]
fn test_dual_trigger_completion_and_milestone_close_both_detect() {
    // When text contains both "completed" and first-person milestone claim,
    // both detection functions fire — but the post-condition chain's
    // sequential ordering means only #4 (completion-claim) fires first.
    let text = "I completed milestone#17 and closed it";
    let summaries: Vec<ToolCallSummary> = vec![];

    // Completion-claim detector: fires on "completed".
    assert!(detect_completion_claim(text).is_some());
    // Milestone-close detector: fires on first-person "completed...milestone".
    assert!(detect_milestone_close_claim_without_patch(text, &summaries).is_some());
}

#[test]
fn test_milestone_close_fires_after_completion_claim_satisfied() {
    // Agent called update_task_status (satisfies guard #4) but no PATCH.
    let text = "I completed milestone#17 and closed it";
    let summaries: Vec<ToolCallSummary> = vec![ToolCallSummary {
        step: 1,
        name: "update_task_status".to_string(),
        input_summary: r#"{"task_id":"abc","status":"completed"}"#.to_string(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];

    assert!(detect_completion_claim(text).is_some());
    // Milestone-close: still fires because no PATCH call in summaries.
    assert!(detect_milestone_close_claim_without_patch(text, &summaries).is_some());
}

#[test]
fn test_detect_milestone_close_claim_ive_form() {
    // "I've closed" should match.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I've closed milestone#14 on GitHub",
        &summaries,
    );
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_we_form() {
    // "We completed" should match.
    let summaries = vec![];
    let result =
        detect_milestone_close_claim_without_patch("We completed milestone#15 today", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_weve_form() {
    // "We've closed out" should match.
    let summaries = vec![];
    let result =
        detect_milestone_close_claim_without_patch("We've closed out milestone#20", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_closed_out_form() {
    // "I closed out" should match.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I closed out the milestone for mika#789",
        &summaries,
    );
    assert!(result.is_some());
}

// -- extract_claimed_milestone_number tests (#1207) --

#[test]
fn test_extract_claimed_milestone_number_with_hash() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone#14"),
        Some(14)
    );
}

#[test]
fn test_extract_claimed_milestone_number_without_hash() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone 14"),
        Some(14)
    );
}

#[test]
fn test_extract_claimed_milestone_number_with_space_hash() {
    assert_eq!(extract_claimed_milestone_number("milestone # 14"), Some(14));
}

#[test]
fn test_extract_claimed_milestone_number_no_number() {
    assert_eq!(
        extract_claimed_milestone_number("I closed the milestone"),
        None
    );
}

#[test]
fn test_extract_claimed_milestone_number_multiple_takes_first() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone#14 not milestone#789"),
        Some(14)
    );
}

// -- PATCH set extraction tests (#1207) --

#[test]
fn test_patch_set_single_patch() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#,
    )];
    // Claim matches the PATCH number → suppressed.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17", &summaries,).is_none()
    );
}

#[test]
fn test_patch_set_multiple_patches() {
    let summaries = vec![
        run_gh_summary(
            r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/14","-f","state=closed"]}"#,
        ),
        run_gh_summary(
            r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/789","-f","state=closed"]}"#,
        ),
    ];
    // Claimed #14, #14 is in the PATCH set → suppressed.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_none()
    );
}

#[test]
fn test_patch_set_no_patches() {
    let summaries = vec![];
    // No patches, first-person claim → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_patch_set_malformed_url_skipped() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/milestones/notanumber","-f","state=closed"]}"#,
    )];
    // Malformed URL doesn't contribute to PATCH set → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_patch_set_non_milestones_path_skipped() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/issues/14","-f","state=closed"]}"#,
    )];
    // PATCH to issues, not milestones → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

// -- Cross-milestone discrimination tests (#1207) --

#[test]
fn test_cross_milestone_claim_different_number_fires() {
    // AC5 from #1207: claim milestone#14 but PATCH milestone#789 → fires.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/789","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_no_number_claim_with_patch_suppressed() {
    // Claim without number + any PATCH → suppressed (presence/absence fallback).
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed the milestone on GitHub", &summaries,)
            .is_none()
    );
}

#[test]
fn test_no_number_claim_without_patch_fires() {
    // Claim without number + no PATCH → fires.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch("I closed the milestone on GitHub", &summaries,)
            .is_some()
    );
}

// -- parse_run_gh_milestone_close_argv unit tests (#1182) --

#[test]
fn test_parse_run_gh_milestone_close_argv_valid() {
    let input =
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/42","-f","state=closed"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), Some(42));
}

#[test]
fn test_parse_run_gh_milestone_close_argv_not_api() {
    let input = r#"{"command":["pr","comment","--body","text"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_no_state_closed() {
    // PATCH without state=closed should not match.
    let input =
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","title=new name"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_truncated_json() {
    // Truncated JSON should return None (graceful fallback).
    let input = r#"{"command":["api","-X","PATCH","/repos/o/r/milest"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_long_method_flag() {
    // "--method" is the long form of "-X" in gh api.
    let input =
        r#"{"command":["api","--method","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), Some(17));
}

// -- milestone-close guard hardening integration tests (#1182) --

#[test]
fn test_milestone_close_guard_substring_spoof_rejected() {
    // A pr comment whose body contains all four substrings should NOT
    // satisfy the guard — the PATCH is in the body text, not an actual
    // api PATCH call.
    let spoofed = run_gh_summary(
        r#"{"command":["pr","comment","--body","closed via PATCH /repos/senara-solutions/mika/milestones/17 state=closed"]}"#,
    );
    let summaries = vec![spoofed];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // Guard should fire — the spoof should not satisfy it.
    assert!(
        result.is_some(),
        "substring spoof should not satisfy the guard"
    );
}

#[test]
fn test_milestone_close_guard_cross_milestone_leakage() {
    // PATCH for milestone#17 should NOT satisfy a claim about milestone#18.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#18 on GitHub", &summaries);
    assert!(
        result.is_some(),
        "PATCH for #17 should not satisfy claim about #18"
    );
}

#[test]
fn test_milestone_close_guard_truncated_input_still_resolves() {
    // Build an argv where state=closed is beyond byte 200 (truncation boundary).
    // The structured parse fails on truncated JSON, but the path and "PATCH"
    // appear before byte 200, so the substring fallback extracts the number.
    let long_org = "a".repeat(150); // Push state=closed past byte 200
    let input = format!(
        r#"{{"command":["api","-X","PATCH","/repos/{}/mika/milestones/17","-f","state=closed"]}}"#,
        long_org
    );
    // Truncate to crate::planning::policy::INPUT_SUMMARY_MAX to simulate what ToolCallSummary does.
    let truncated =
        crate::tool_execution::truncate_summary(&input, crate::planning::policy::INPUT_SUMMARY_MAX);
    let summaries = vec![run_gh_summary(&truncated)];

    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // If state=closed was truncated, substring fallback also fails →
    // guard correctly fires (over-fire is the safe direction per the ticket).
    // The key invariant: the guard does NOT stall — it either suppresses
    // (if substring fallback finds a match) or fires (if not).
    // With 150-char org name, state=closed is truncated away, so the guard fires.
    assert!(
        result.is_some(),
        "truncated state=closed should cause guard to fire (safe direction)"
    );
}

#[test]
fn test_milestone_close_guard_long_method_flag() {
    // "--method" is the long form of "-X" in gh api.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","--method","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries,)
            .is_none(),
        "--method PATCH should satisfy the guard"
    );
}

#[test]
fn test_milestone_close_guard_non_json_fallback() {
    // If input_summary is not valid JSON (e.g., pre-existing format or
    // corruption), the substring fallback should still work.
    let summaries = vec![run_gh_summary(
        r#"api -X PATCH /repos/senara-solutions/mika/milestones/17 -f state=closed "api" "PATCH""#,
    )];
    // This is a non-JSON string that happens to contain the substring markers.
    // The structured parse fails; the substring fallback should try to extract.
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // Substring fallback finds "api", "PATCH", and "state=closed" as substrings,
    // plus the milestone path regex matches. Should suppress the guard.
    assert!(
        result.is_none(),
        "substring fallback should work for non-JSON input"
    );
}
