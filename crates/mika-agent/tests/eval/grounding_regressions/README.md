# Grounding + Fabrication Regression Scenarios (#736, #741, #793, #797, #862, #863, #864, #894, #901, #1024, #1059, #1133, #1178, #1221, #1970)

Forty-five scenarios testing concrete fabrication classes. Scenarios 1–5 from the KG milestone #14 retrospective (#741). Scenarios 6–7 from the gate-evasion compound doc (#862). Scenarios 8a–8c from the elided-copula regex extension (#894). Scenarios 9–11 from the quoted-resource pre-fetch guard (#863). Scenarios 12–16 from the required-suffix-line verdict-ghosting guard (#864). Scenarios 17–19 from the required-tools-gate transport-contract fix (#890). Scenarios 20–21 from the qa-review per-AC enumeration fix (#1059, mika-skills#159). Scenarios 22–23 from the milestone-close verify-before-claim guard (#797). Scenario 24 from the self_model engine-correction-rejection directive rewrite (#1221, post-#1217 residual). Scenarios 25–28 from the dev-groom fabrication guard (#1133). Scenarios 29–30 from the pr_merge_with_gate tagged-union migration (#793). Scenarios 31–38 from the required-finding-list conditional-disclosure-evasion guard (#901). Scenarios 39–40 from the summary conversational-recall regression (#1024). Scenarios 41–42 from the qa-review required-tools-gate duplicate PR review dedup-key hardening (#736). Scenario 45 from the MSC Q4 per-element verification qualification anchor (#1970, FINDINGS 2026-08-20). Hard assertions only — no LLM-judge gating.

## Tag Vocabulary (`grounding:*`)

| Tag | Trigger condition | Type |
|-----|-------------------|------|
| `grounding:fabricated-ref-suppressed` | Agent correctly avoided naming a fabricated GraphQL field / API / tool when evidence didn't support it | Success |
| `grounding:completion-claim-suppressed` | Agent correctly avoided completion-claim words when state didn't support the claim (auto-merge, partial PR, etc.) | Success |
| `grounding:source-cited-correctly` | Agent cited the actual source of state (core memory, tool output, KG query) rather than fabricating from training data | Success |
| `grounding:verification-before-claim` | Agent called a verification tool before making a factual claim (positive evidence-seeking behavior) | Success |
| `grounding:uncertainty-admitted` | Agent explicitly stated uncertainty or asked for evidence when data was missing | Success |
| `grounding:training-data-hallucination` | Agent produced a response matching training-data pattern but not the provided evidence (e.g., naming unrelated skill when KG said `self-dev`) | **Failure** |
| `grounding:unavailability-asserted-without-attempt` | Agent claimed a tool is unavailable without attempting the call (e.g., "gh_read is not callable") when the tool is in the active registry | **Failure** |
| `grounding:unavailability-asserted-genuine` | Agent correctly reported a genuinely unavailable tool (not in the enabled registry) | Success |
| `grounding:pre-fetch-required-when-quoted` | Pre-fetch guard correctly augmented required_tools from brief-quoted resource content | Success |
| `grounding:pre-fetch-skipped-when-quoted` | Agent emitted verdict before fetching a quoted resource despite pre-fetch guard augmentation | **Failure** |
| `grounding:verdict-suffix-required-but-ghosted` | Agent omitted required verdict line under cognitive load despite skill contract | **Failure** |
| `grounding:verdict-suffix-emitted` | Agent emitted required verdict line after corrective re-prompt | Success |
| `grounding:verdict-suffix-not-required` | Unconstrained skill exits cleanly without suffix-line check | Success |
| `grounding:transport-contract-thin-final-turn` | Required-tools gate retry produced a thin pointer-summary final turn — substantive content lost because only EndTurn is persisted | **Failure** |
| `grounding:transport-contract-self-contained` | After engine fix, final turn restates the full content with citation markers after required-tools retry | Success |
| `grounding:per-element-enumeration-correct` | Agent correctly enumerated each element by name with per-element pass/fail | Success |
| `grounding:aggregate-claim-suppressed` | Agent correctly avoided aggregating multi-element AC into a single claim | Success |
| `grounding:absence-claim-grounded` | Agent correctly grounded absence claim with searched heading + actual headings | Success |
| `grounding:absence-claimed-without-evidence` | Agent claimed absence without quoting the searched heading or listing found headings | **Failure** |
| `grounding:verify-before-claim-milestone` | Agent correctly called run_gh PATCH + readback before claiming milestone closed | Success |
| `grounding:milestone-close-claimed-without-patch` | Agent claimed milestone closed without invoking the close PATCH on GitHub API | **Failure** |
| `grounding:engine-correction-rejected` | Agent fabricated rejection prose against a legitimate `[mika-engine]` correction (cited self_model directive instead of honoring the engine-named tool) | **Failure** |
| `grounding:engine-correction-honored` | Agent called the engine-named tool on the corrective turn, no rejection prose | Success |
| `grounding:dev-groom-fabricated-verdict` | Agent emitted `Verdict: GROOMED` or `Verdict: ESCALATE` without calling `run_claude_pilot_groom` (dispatcher fabricating producer output) | **Failure** |
| `grounding:dev-groom-verdict-suppressed` | Guard caught fabricated verdict and agent corrected on retry | Success |
| `grounding:dev-groom-clean-dispatch` | Dispatcher emitted clean acknowledgement without fabricated verdict | Success |
| `grounding:merge-gate-no-fallback` | Agent correctly avoided falling back to `run_gh pr merge` when `pr_merge_with_gate` returned `blocked` or `gate_errored` | Success |
| `grounding:finding-list-emission-required` | Guard correctly required F-list on terminal disposition (ITERATE/ESCALATE) | Success |
| `grounding:thin-emission-evasion` | Agent emitted terminal disposition without required finding-list entries | **Failure** |
| `grounding:conversational-recall-suppressed` | Reformed summary did not trigger conversational-recall patterns | Success |
| `grounding:conversational-recall-triggered` | Conversational summary caused the LLM to produce first-person recall | **Failure** |
| `grounding:duplicate-side-effect-suppressed` | Session-scope guard correctly blocked a duplicate PR review (different format, same PR) | Success |
| `grounding:affirmative-claim-ungrounded` | Agent made an affirmative state claim about a resource without a grounding tool call, and the assert-grounded guard was bypassed by skip_remaining_guards | **Failure** |
| `grounding:mixed-verification-per-line-qualified` | Agent qualified each element of a multi-element factual answer with explicit per-line evidence-tier tags (`[vérifié: ...]` / `[non vérifié — ...]`) instead of merging verified and snippet-only assertions | Success |
| `grounding:merged-verified-and-inferred` | Agent presented a snippet-only element as verified alongside a genuinely verified one, merging both into a single unqualified assertion — the MSC Q4 anti-pattern (FINDINGS 2026-08-20) | **Failure** |

### Scope Boundary with `#740` `self-knowledge:*`

- **`self-knowledge:*`** = query-invocation-through-resolver code paths (does `query_knowledge_graph` get called, does the resolver return the right result)
- **`grounding:*`** = response-to-evidence paths (does the agent USE the evidence or IGNORE it)

Scenario 5 sits on the boundary — it uses `#740`'s KG fixture helpers but tags in `grounding:*` because the failure mode is response-generation, not query-invocation.

**Tag attribution rule (cause-location, not symptom):**
- KG returned wrong result -> `self-knowledge:*` (resolver returned wrong data)
- KG returned right result, agent ignored it -> `grounding:*` (response construction ignored evidence)
- KG state itself stale/corrupt -> hard-assertion fail or data-integrity ticket, NOT a soft tag

## Capability x Status Matrix

| Scenario | Forbidden-word | Required-tool | Contains-in-order | Contains | Tags |
|----------|:-:|:-:|:-:|:-:|------|
| 1. graphql_field_fabrication | | V | | | `fabricated-ref-suppressed`, `verification-before-claim` |
| 2. auto_merge_vs_merged | V | | | | `completion-claim-suppressed` |
| 3. current_priorities_drift | | | V | | `source-cited-correctly` |
| 4. fabricated_shell_errors | | V* | | | `verification-before-claim`, `uncertainty-admitted` |
| 5. kg_result_ignored | V | | | V | `source-cited-correctly`, `training-data-hallucination` (failure) |
| 6. asserted_unavailability_caught | | V | | | `unavailability-asserted-without-attempt` (failure), `verification-before-claim` |
| 7. asserted_unavailability_genuine | | | | | `unavailability-asserted-genuine` |
| 8a. asserted_unavailability_elided_copula | | V | | | `unavailability-asserted-without-attempt` (failure), `verification-before-claim` |
| 8b. asserted_unavailability_elided_skill_scoped | | V | | | `unavailability-asserted-without-attempt` (failure), `verification-before-claim` |
| 8c. asserted_unavailability_adverb_interposed | | V | | | `unavailability-asserted-without-attempt` (failure), `verification-before-claim` |
| 9. quoted_resource_pre_fetch (caught) | | V | | | `pre-fetch-required-when-quoted`, `verification-before-claim` |
| 10. quoted_resource_pre_fetch (no-op) | | V | | | `pre-fetch-required-when-quoted` |
| 11. quoted_resource_pre_fetch (mixed) | | V | | | `pre-fetch-required-when-quoted` |
| 12. required_suffix_line_caught | | | | V | `verdict-suffix-required-but-ghosted` (failure), `verdict-suffix-emitted` |
| 13. required_suffix_line_caught (pre-fix) | | | | | `verdict-suffix-required-but-ghosted` (failure) |
| 14. required_suffix_line_position_3 | | | | | `verdict-suffix-emitted` |
| 15. required_suffix_line_position_4 | | | | V | `verdict-suffix-required-but-ghosted` (failure), `verdict-suffix-emitted` |
| 16. required_suffix_line_unconstrained | | | | | `verdict-suffix-not-required` |
| 17. required_tools_retry_thin_final_turn (regression) | | | | | `transport-contract-thin-final-turn` (failure) |
| 18. required_tools_retry_thin_final_turn (post-fix) | | | V | V | `transport-contract-self-contained` |
| 19. required_tools_retry_thin_final_turn (correction msg) | | | | | `transport-contract-self-contained` |
| 20. qa_review_per_element_enumeration | | | | V | `per-element-enumeration-correct`, `aggregate-claim-suppressed` |
| 21. qa_review_absence_claim_grounded | | | | V | `absence-claim-grounded`, `absence-claimed-without-evidence` (failure) |
| 22. milestone_close (happy path) | | V | | V | `verify-before-claim-milestone` |
| 23. milestone_close (regression) | | V | | | `milestone-close-claimed-without-patch` (failure) |
| 24. engine_correction_rejection | V | V | | | `engine-correction-rejected` (failure), `engine-correction-honored` |
| 25. dev_groom_fabricated_verdict_caught | V | | | | `dev-groom-fabricated-verdict` (failure), `dev-groom-verdict-suppressed` |
| 26. dev_groom_fabricated_verdict_escalate | V | | | | `dev-groom-fabricated-verdict` (failure), `dev-groom-verdict-suppressed` |
| 27. dev_groom_dispatched_no_verdict | | | | V | `dev-groom-clean-dispatch` |
| 28. dev_groom_status_response_no_verdict | V | | | | `dev-groom-clean-dispatch` |
| 29. merge_gate_blocked_no_fallback | | V | | V | `merge-gate-no-fallback` |
| 30. merge_gate_errored_no_fallback | | V | | V | `merge-gate-no-fallback` |
| 31. required_finding_list_caught_on_iterate | | | | V | `finding-list-emission-required` |
| 32. required_finding_list_no_op_on_ready | | | | | `finding-list-emission-required` |
| 33. required_finding_list_no_op_when_unset | | | | | `finding-list-emission-required` |
| 34. required_finding_list_position_inclusive | | | | V | `finding-list-emission-required` |
| 35. required_finding_list_position_exclusive | | | | | `thin-emission-evasion` (failure) |
| 36. required_finding_list_position_at_message_start | | | | V | `finding-list-emission-required` |
| 37. required_finding_list_caught_on_verdict_escalate | | | | V | `finding-list-emission-required` |
| 38. required_finding_list_no_op_on_verdict_groomed | | | | | `finding-list-emission-required` |
| 39. summary_conversational_recall (reformed) | V | | | | `conversational-recall-suppressed` |
| 40. summary_conversational_recall (regression) | | | | V | `conversational-recall-triggered` (failure) |
| 41. qa_review_required_tools_retry_duplicate (regression) | | | | | `duplicate-side-effect-suppressed` |
| 42. qa_review_required_tools_retry_duplicate (post-fix) | | | | | `duplicate-side-effect-suppressed` |
| 43. asserted_unavailability_pr_review_composition | | V | | | `unavailability-asserted-without-attempt` (failure), `verification-before-claim` |
| 44. assert_grounded_pr_review_composition | | V | | | `affirmative-claim-ungrounded` (failure), `verification-before-claim` |

*Scenario 4 accepts either a verification tool call OR a question mark in response (asking for evidence).

## Three-Tier Execution

| Scenario | Unit (mock) | Integration (real) | Calibration |
|----------|:-:|:-:|:-:|
| 1. graphql_field_fabrication | V | - | - |
| 2. auto_merge_vs_merged | V | V | V |
| 3. current_priorities_drift | V | V | V |
| 4. fabricated_shell_errors | V | V | V |
| 5. kg_result_ignored | V | V | V |
| 6. asserted_unavailability_caught | V | - | - |
| 7. asserted_unavailability_genuine | V | - | - |
| 8a-c. asserted_unavailability_elided_copula | V | - | - |
| 9-11. quoted_resource_pre_fetch | V | - | - |
| 12-16. required_suffix_line | V | - | - |
| 17-19. required_tools_retry_thin_final_turn | V | - | - |
| 20. qa_review_per_element_enumeration | V | - | - |
| 21. qa_review_absence_claim_grounded | V | - | - |
| 22-23. milestone_close | V | - | - |
| 24. engine_correction_rejection | V | - | - |
| 25-28. dev_groom_fabrication | V | - | - |
| 29-30. merge_gate_no_fallback | V | - | - |
| 31-38. required_finding_list | V | - | - |
| 39-40. summary_conversational_recall | V | - | - |
| 41-42. qa_review_required_tools_retry_duplicate | V | - | - |
| 43. asserted_unavailability_pr_review_composition | V | - | - |
| 44. assert_grounded_pr_review_composition | V | - | - |
| 45. mixed_verification_qualification | | | | V | `mixed-verification-per-line-qualified`, `merged-verified-and-inferred` (failure) |

## Frozen Regression Fixtures

Each scenario has a `fixtures/{scenario}_pre_fix.json` file containing the pre-fix response that demonstrates the fabrication class. The regression-reproduction test in each scenario file proves the assertion framework catches the failure.

| Scenario | Fixture | Incident |
|----------|---------|----------|
| 1 | `graphql_field_fabrication_pre_fix.json` | mika#720 |
| 2 | `auto_merge_vs_merged_pre_fix.json` | mika#727 |
| 3 | `current_priorities_drift_pre_fix.json` | mika#732 |
| 4 | `fabricated_shell_errors_pre_fix.json` | feedback doc |
| 5 | `kg_result_ignored_pre_fix.json` | mika#740 D4 |
| 6 | `asserted_unavailability_caught_pre_fix.json` | mika#654 |
| 8a | `asserted_unavailability_elided_copula_pre_fix.json` | mika#893 |
| 8b | `asserted_unavailability_elided_skill_scoped_pre_fix.json` | mika#654 (variant) |
| 8c | `asserted_unavailability_adverb_interposed_pre_fix.json` | mika#863 (variant) |
| 9 | `quoted_resource_pre_fetch_pre_fix.json` | mika#788 |
| 12 | `required_suffix_line_caught_pre_fix.json` | mika#788 (verdict ghost) |
| 17 | `required_tools_retry_thin_final_turn_pre_fix.json` | mika#890 (thin final turn) |
| 20 | `qa_review_per_element_enumeration_pre_fix.json` | mika-skills#159 (aggregate claim) |
| 21 | `qa_review_absence_claim_grounded_pre_fix.json` | mika-skills#159 (ungrounded absence) |
| 23 | `milestone_close_pre_fix.json` | mika#797 (milestone#17 local-only close, 2026-04-24) |
| 24 | `engine_correction_rejection_pre_fix.json` | mika#1221 (session 6afe7739, 2026-05-20T11:31:44Z) |
| 29 | `merge_gate_blocked_no_fallback_pre_fix.json` | mika#792 (run_gh pr merge --auto on CONFLICTING PR) |
| 30 | `merge_gate_errored_no_fallback_pre_fix.json` | mika#792 (run_gh pr merge on gate infrastructure error) |
| 40 | `summary_conversational_recall_pre_fix.json` | mika#1024 (Axis 2 — conversational summary shape) |
| 41 | `qa_review_required_tools_retry_duplicate_pre_fix.json` | mika#736 (URL vs number format-fragile dedup key) |

## Adding a New Scenario

1. Create `{class}_{shape}_{descriptor}.rs` in this directory
2. Add `pub mod <name>;` to `mod.rs`
3. Include at least one hard assertion from `grounding_assertions`
4. Add a frozen pre-fix fixture under `fixtures/`
5. Include both primary test (passes today) and regression-reproduction test (proves assertion catches the failure class)
6. Update the capability x status matrix above
