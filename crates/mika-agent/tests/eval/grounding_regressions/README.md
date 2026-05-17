# Grounding + Fabrication Regression Scenarios (#741, #862, #863, #864, #894, #1059)

Twenty-three scenarios testing concrete fabrication classes. Scenarios 1–5 from the KG milestone #14 retrospective (#741). Scenarios 6–7 from the gate-evasion compound doc (#862). Scenarios 8a–8c from the elided-copula regex extension (#894). Scenarios 9–11 from the quoted-resource pre-fetch guard (#863). Scenarios 12–16 from the required-suffix-line verdict-ghosting guard (#864). Scenarios 17–19 from the required-tools-gate transport-contract fix (#890). Scenarios 20–21 from the qa-review per-AC enumeration fix (#1059, mika-skills#159). Hard assertions only — no LLM-judge gating.

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

## Adding a New Scenario

1. Create `{class}_{shape}_{descriptor}.rs` in this directory
2. Add `pub mod <name>;` to `mod.rs`
3. Include at least one hard assertion from `grounding_assertions`
4. Add a frozen pre-fix fixture under `fixtures/`
5. Include both primary test (passes today) and regression-reproduction test (proves assertion catches the failure class)
6. Update the capability x status matrix above
