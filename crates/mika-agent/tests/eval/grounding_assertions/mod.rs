//! # Grounding Assertion Helpers (#741)
//!
//! Crate-shared assertion helpers for fabrication-detection scenarios.
//! Each helper targets a specific grounding failure shape:
//!
//! - **Forbidden-word assertion:** response text MUST NOT contain specific words
//! - **Required-tool assertion:** specific tool(s) MUST be called before EndTurn
//! - **Ordered-content assertion:** response MUST contain items in a specific order
//!
//! These are hard assertions (test-gating), not soft tags.
//!
//! ## Design
//!
//! Per plan D1: two hard-assertion shapes, composable per scenario.
//! No LLM-judge gating — each fabrication class has objectively checkable signals.
//!
//! Reference: mika#741 D1

use super::trace::AgentTrace;

/// Assert that the agent's response text does NOT contain any of the forbidden words.
///
/// Normalization: case-insensitive, strips surrounding punctuation from each word
/// in the response before matching. The forbidden list is scenario-specific.
///
/// # Panics
/// Panics with a descriptive message listing each forbidden word found and its
/// location in the response.
pub fn assert_response_forbids(trace: &AgentTrace, forbidden: &[&str]) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let normalized = text.to_lowercase();

    let mut violations = Vec::new();
    for &word in forbidden {
        let word_lower = word.to_lowercase();
        // Check for word boundary matches — the forbidden word must appear as a
        // standalone word or at a word boundary, not as a substring of another word.
        // We split on whitespace and punctuation-strip each token.
        for token in normalized.split_whitespace() {
            let stripped = token.trim_matches(|c: char| !c.is_alphanumeric());
            if stripped == word_lower {
                violations.push((word, token.to_string()));
            }
        }
    }

    if !violations.is_empty() {
        let detail: Vec<String> = violations
            .iter()
            .map(|(word, ctx)| format!("  forbidden '{}' found as '{}'", word, ctx))
            .collect();
        panic!(
            "assert_response_forbids failed:\n  response: {:?}\n  violations:\n{}",
            truncate(text, 300),
            detail.join("\n")
        );
    }
}

/// Assert that a specific tool was called at some point before the final EndTurn response.
///
/// Checks the tool call trace for the named tool. This is a hard assertion that
/// the agent sought evidence before making a claim.
///
/// # Panics
/// Panics if the named tool was not called during the run.
pub fn assert_tool_called_before_response(trace: &AgentTrace, tool_name: &str) {
    let called: Vec<&str> = trace.tool_names();
    if !called.contains(&tool_name) {
        panic!(
            "assert_tool_called_before_response failed:\n  expected tool '{}' to be called\n  actual tools: {:?}",
            tool_name, called
        );
    }
}

/// Assert that at least one tool from the given set was called before EndTurn.
///
/// Used when multiple tools could satisfy the verification requirement
/// (e.g., `build_mika` OR `run_gh` OR `read_file` for error verification).
///
/// # Panics
/// Panics if none of the listed tools were called during the run.
pub fn assert_any_tool_called_from(trace: &AgentTrace, tool_names: &[&str]) {
    let called: Vec<&str> = trace.tool_names();
    let found = tool_names.iter().any(|t| called.contains(t));

    if !found {
        panic!(
            "assert_any_tool_called_from failed:\n  expected at least one of {:?}\n  actual tools: {:?}",
            tool_names, called
        );
    }
}

/// Assert that the response contains all items in the given order.
///
/// Each item must appear in the response text, and their positions must be
/// monotonically increasing. Case-insensitive matching.
///
/// # Panics
/// Panics if any item is missing or items appear out of order.
pub fn assert_response_contains_in_order(trace: &AgentTrace, items: &[&str]) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();

    let mut search_start: usize = 0;
    for (i, &item) in items.iter().enumerate() {
        let item_lower = item.to_lowercase();
        match lower[search_start..].find(&item_lower) {
            Some(offset) => {
                // Advance past the END of this match so the next item
                // cannot overlap with this one.
                search_start = search_start + offset + item_lower.len();
            }
            None => {
                panic!(
                    "assert_response_contains_in_order failed at item {} ({:?}):\n  \
                     items: {:?}\n  response: {:?}\n  searched from byte position {}",
                    i,
                    item,
                    items,
                    truncate(text, 300),
                    search_start,
                );
            }
        }
    }
}

/// Assert that the response contains a question mark (agent is asking for evidence).
///
/// Used as an alternative acceptance criterion in scenario 4: if the agent
/// doesn't verify via a tool, asking for evidence is also acceptable.
pub fn assert_response_contains_question(trace: &AgentTrace) {
    let text = trace.output.text.as_deref().unwrap_or("");
    if !text.contains('?') {
        panic!(
            "assert_response_contains_question failed:\n  response: {:?}",
            truncate(text, 300)
        );
    }
}

/// Assert that the response text contains a specific substring (case-insensitive).
///
/// Unlike `assertions::assert_output_contains` which is case-sensitive, this
/// performs case-insensitive substring matching — appropriate for grounding
/// checks where the agent may vary casing.
pub fn assert_response_contains(trace: &AgentTrace, expected: &str) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();
    let expected_lower = expected.to_lowercase();
    if !lower.contains(&expected_lower) {
        panic!(
            "assert_response_contains failed:\n  expected to contain: {:?}\n  response: {:?}",
            expected,
            truncate(text, 300)
        );
    }
}

/// Assert that the response contains per-element enumeration for each named element.
///
/// Checks that each element name appears in the response text and is followed
/// (within a reasonable window) by a pass/fail indicator: `✓`, `✗`, `pass`, or `fail`.
/// This enforces the per-element enumeration rule from qa-review Step 2.5.5:
/// "Enumerate every element by name with its observed value. Never aggregate."
///
/// # Panics
/// Panics if any named element is missing from the response or lacks a pass/fail indicator.
pub fn assert_response_contains_per_element_enumeration(trace: &AgentTrace, elements: &[&str]) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();

    let mut missing_elements = Vec::new();
    let mut missing_indicators = Vec::new();

    for &element in elements {
        let element_lower = element.to_lowercase();
        match lower.find(&element_lower) {
            None => {
                missing_elements.push(element);
            }
            Some(pos) => {
                // Look for a pass/fail indicator within 200 chars after the element name.
                // Operate entirely on `lower` to avoid byte-offset mismatches between
                // `lower` and `text` when to_lowercase() expands multi-byte characters.
                let search_start = pos + element_lower.len();
                let search_end = (search_start + 200).min(lower.len());
                // Find safe UTF-8 boundary within `lower`
                let mut end = search_end;
                while end > search_start && !lower.is_char_boundary(end) {
                    end -= 1;
                }
                let window = &lower[search_start..end];

                let has_indicator = window.contains('✓')
                    || window.contains('✗')
                    || window.contains("pass")
                    || window.contains("fail");

                if !has_indicator {
                    missing_indicators.push(element);
                }
            }
        }
    }

    if !missing_elements.is_empty() || !missing_indicators.is_empty() {
        let mut detail = Vec::new();
        for &el in &missing_elements {
            detail.push(format!("  element {:?} not found in response", el));
        }
        for &el in &missing_indicators {
            detail.push(format!(
                "  element {:?} found but no pass/fail indicator nearby",
                el
            ));
        }
        panic!(
            "assert_response_contains_per_element_enumeration failed:\n  \
             expected per-element enumeration for {:?}\n{}\n  response: {:?}",
            elements,
            detail.join("\n"),
            truncate(text, 500),
        );
    }
}

/// Assert that absence claims in the response are grounded with evidence.
///
/// When the response claims content is absent, it must include:
/// 1. The searched heading text
/// 2. A list of actual headings found (indicated by "sections:", "headings found:",
///    "not present in", or similar phrasing)
///
/// The function first checks if the response contains an absence-claim keyword
/// ("not present", "missing", "absent", "could not find", "does not appear",
/// "no section"). If an absence claim is detected, it verifies the searched
/// heading and evidence list are present.
///
/// # Panics
/// Panics if an absence claim is detected but the searched heading or evidence
/// list is missing from the response.
pub fn assert_absence_claim_grounded(trace: &AgentTrace, searched_heading: &str) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();

    // Absence-claim keywords (conservative set — extend as new phrasings are observed)
    let absence_keywords = [
        "not present",
        "missing",
        "absent",
        "could not find",
        "does not appear",
        "no section",
    ];

    let has_absence_claim = absence_keywords.iter().any(|kw| lower.contains(kw));

    if !has_absence_claim {
        // No absence claim detected — nothing to ground
        return;
    }

    // Check 1: the searched heading must be mentioned
    let heading_lower = searched_heading.to_lowercase();
    if !lower.contains(&heading_lower) {
        panic!(
            "assert_absence_claim_grounded failed:\n  \
             absence claim detected but searched heading {:?} not found in response\n  \
             response: {:?}",
            searched_heading,
            truncate(text, 500),
        );
    }

    // Check 2: evidence of actual headings found (list of sections)
    let evidence_markers = [
        "sections:",
        "headings found:",
        "not present in",
        "section headings",
        "pr body sections:",
    ];
    let has_evidence = evidence_markers.iter().any(|marker| lower.contains(marker));

    if !has_evidence {
        panic!(
            "assert_absence_claim_grounded failed:\n  \
             absence claim detected with heading {:?} but no evidence list of actual \
             headings/sections found in response\n  \
             response: {:?}",
            searched_heading,
            truncate(text, 500),
        );
    }
}

/// Verification tier declared for an element in the per-line qualification assertion.
///
/// Used by [`assert_per_line_verification_qualification`] to enforce that each
/// element in a multi-element response carries a bracketed evidence-tier tag
/// matching the declared tier.
#[derive(Debug, Clone, Copy)]
pub enum VerificationTier<'a> {
    /// Element is verified by a named source (e.g., a page opened by a tool call).
    /// The response must carry a `[vérifié: <source>]` (or `[verified: <source>]`)
    /// tag adjacent to the element.
    Verified(&'a str),
    /// Element is only supported by snippet convergence, not verified at source.
    /// The response must carry a `[non vérifié ...]` (or `[unverified ...]`) tag
    /// adjacent to the element AND must NOT carry a `[vérifié: ...]` tag on it.
    SnippetOnly,
}

/// Assert that each element in a multi-element response carries a bracketed
/// evidence-tier qualification tag matching its declared verification tier.
///
/// For each `(element, tier)`:
/// - The element name MUST appear in the response (case-insensitive).
/// - Within a bounded window (200 chars) after the element name, a bracketed
///   qualification tag MUST appear whose tier matches the declared tier:
///   - `Verified(source)` — window must contain `[vérifié:` (or `[verified:`).
///   - `SnippetOnly` — window must contain `[non vérifié` (or `[unverified`)
///     AND must NOT contain `[vérifié:` (nor `[verified:`) — a snippet-only
///     element tagged as verified is the anti-pattern this check catches.
///
/// This helper enforces the shape catalogued by tags
/// `grounding:mixed-verification-per-line-qualified` (success) and
/// `grounding:merged-verified-and-inferred` (failure) — the MSC Q4 founding
/// class (see `mixed_verification_qualification.rs`).
///
/// Case-insensitive matching; UTF-8 boundary safe.
///
/// # Panics
/// Panics with a descriptive message listing each element that was missing
/// from the response, missing a tier tag, or carrying a mis-matched tier tag.
pub fn assert_per_line_verification_qualification(
    trace: &AgentTrace,
    elements: &[(&str, VerificationTier<'_>)],
) {
    let text = trace.output.text.as_deref().unwrap_or("");
    let lower = text.to_lowercase();

    let mut violations = Vec::new();

    for &(element, tier) in elements {
        let element_lower = element.to_lowercase();
        let pos = match lower.find(&element_lower) {
            Some(p) => p,
            None => {
                violations.push(format!("element {:?} not found in response", element));
                continue;
            }
        };

        // Bounded 200-char window after the element name, UTF-8 boundary safe.
        let search_start = pos + element_lower.len();
        let search_end = (search_start + 200).min(lower.len());
        let mut end = search_end;
        while end > search_start && !lower.is_char_boundary(end) {
            end -= 1;
        }
        let window = &lower[search_start..end];

        let has_verified_tag = window.contains("[vérifié:") || window.contains("[verified:");
        let has_unverified_tag = window.contains("[non vérifié") || window.contains("[unverified");

        match tier {
            VerificationTier::Verified(source) => {
                if !has_verified_tag {
                    violations.push(format!(
                        "element {:?} declared Verified({:?}) but no `[vérifié: ...]` \
                         (or `[verified: ...]`) tag found within 200 chars after the element name",
                        element, source
                    ));
                }
            }
            VerificationTier::SnippetOnly => {
                if !has_unverified_tag {
                    violations.push(format!(
                        "element {:?} declared SnippetOnly but no `[non vérifié ...]` \
                         (or `[unverified ...]`) tag found within 200 chars after the element name",
                        element
                    ));
                }
                if has_verified_tag {
                    violations.push(format!(
                        "element {:?} declared SnippetOnly but response carries a \
                         `[vérifié: ...]` (or `[verified: ...]`) tag on it — this is the \
                         `grounding:merged-verified-and-inferred` anti-pattern",
                        element
                    ));
                }
            }
        }
    }

    if !violations.is_empty() {
        let detail: Vec<String> = violations.iter().map(|v| format!("  {}", v)).collect();
        panic!(
            "assert_per_line_verification_qualification failed:\n{}\n  response: {:?}",
            detail.join("\n"),
            truncate(text, 500),
        );
    }
}

/// Truncate a string for display in panic messages.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::trace::AgentTrace;
    use mika_agent::agent::AgentOutput;

    fn make_trace(text: &str, tools: &[&str]) -> AgentTrace {
        use mika_agent::db::ToolCallRow;

        let tool_calls: Vec<ToolCallRow> = tools
            .iter()
            .enumerate()
            .map(|(i, name)| ToolCallRow {
                id: format!("tc_{i}"),
                agent_id: "test".to_string(),
                session_id: "test".to_string(),
                trace_id: Some("test".to_string()),
                llm_call_id: None,
                step: i as u32,
                tool_name: name.to_string(),
                tool_source: "builtin".to_string(),
                skill_name: None,
                input: None,
                output: None,
                success: true,
                non_zero_exit: false,
                latency_ms: 10,
                error_message: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .collect();

        AgentTrace {
            output: AgentOutput {
                text: Some(text.to_string()),
                thinking: None,
                usage: None,
            },
            llm_calls: vec![],
            tool_calls,
            captured_requests: vec![],
            llm_call_count: 1,
        }
    }

    #[test]
    fn forbids_catches_forbidden_word() {
        let trace = make_trace("The PR was merged successfully.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_forbids(&trace, &["merged", "shipped"]);
        });
        assert!(result.is_err(), "Should have panicked on 'merged'");
    }

    #[test]
    fn forbids_passes_when_clean() {
        let trace = make_trace("Auto-merge is enabled; CI still pending.", &[]);
        assert_response_forbids(&trace, &["merged", "shipped", "deployed"]);
    }

    #[test]
    fn forbids_case_insensitive() {
        let trace = make_trace("PR was MERGED.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_forbids(&trace, &["merged"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn any_tool_called_passes() {
        let trace = make_trace("OK", &["build_mika", "search_memory"]);
        assert_any_tool_called_from(&trace, &["build_mika", "run_gh", "read_file"]);
    }

    #[test]
    fn any_tool_called_fails_when_none_match() {
        let trace = make_trace("OK", &["search_memory"]);
        let result = std::panic::catch_unwind(|| {
            assert_any_tool_called_from(&trace, &["build_mika", "run_gh", "read_file"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn contains_in_order_passes() {
        let trace = make_trace("1. Alpha task\n2. Beta task\n3. Gamma task", &[]);
        assert_response_contains_in_order(&trace, &["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn contains_in_order_fails_wrong_order() {
        let trace = make_trace("1. Beta task\n2. Alpha task\n3. Gamma task", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_in_order(&trace, &["Alpha", "Beta", "Gamma"]);
        });
        assert!(result.is_err());
    }

    #[test]
    fn tool_called_before_response_passes() {
        let trace = make_trace("OK", &["run_gh"]);
        assert_tool_called_before_response(&trace, "run_gh");
    }

    #[test]
    fn response_contains_question_passes() {
        let trace = make_trace("Could you provide the build output?", &[]);
        assert_response_contains_question(&trace);
    }

    #[test]
    fn response_contains_case_insensitive() {
        let trace = make_trace("The Self-Dev skill handles PR merges.", &[]);
        assert_response_contains(&trace, "self-dev");
    }

    #[test]
    fn tool_called_before_response_fails_when_absent() {
        let trace = make_trace("OK", &["search_memory"]);
        let result = std::panic::catch_unwind(|| {
            assert_tool_called_before_response(&trace, "nonexistent_tool");
        });
        assert!(result.is_err());
    }

    #[test]
    fn response_contains_question_fails_when_no_question() {
        let trace = make_trace("No question here.", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_question(&trace);
        });
        assert!(result.is_err());
    }

    #[test]
    fn response_contains_fails_when_absent() {
        let trace = make_trace("Hello world", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_response_contains(&trace, "missing phrase");
        });
        assert!(result.is_err());
    }

    #[test]
    fn forbids_ignores_substring_inside_compound_word() {
        // "unmerged" should NOT trigger on "merged" — boundary check prevents it
        let trace = make_trace("The PR is still unmerged.", &[]);
        assert_response_forbids(&trace, &["merged"]);
    }

    // --- Per-element enumeration tests ---

    #[test]
    fn per_element_enumeration_passes_with_all_elements() {
        let trace = make_trace(
            "- mika primary: 70.8% → ✓ pass\n\
             - mika-skills: 52.9% → ✓ pass\n\
             - mika-platform: 47.9% → ✗ fail (below 50%)\n\
             - mika-cloud: 31.2% → ✗ fail (below 50%)",
            &[],
        );
        assert_response_contains_per_element_enumeration(
            &trace,
            &["mika primary", "mika-skills", "mika-platform", "mika-cloud"],
        );
    }

    #[test]
    fn per_element_enumeration_fails_when_element_missing() {
        let trace = make_trace(
            "- mika primary: 70.8% → ✓ pass\n\
             - mika-skills: 52.9% → ✓ pass",
            &[],
        );
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_per_element_enumeration(
                &trace,
                &["mika primary", "mika-skills", "mika-platform"],
            );
        });
        assert!(result.is_err(), "Should panic when element is missing");
    }

    #[test]
    fn per_element_enumeration_fails_when_no_indicator() {
        let trace = make_trace(
            "- mika primary: 70.8%\n\
             - mika-skills: 52.9%",
            &[],
        );
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_per_element_enumeration(
                &trace,
                &["mika primary", "mika-skills"],
            );
        });
        assert!(
            result.is_err(),
            "Should panic when no pass/fail indicator present"
        );
    }

    #[test]
    fn per_element_enumeration_catches_aggregate_claim() {
        // Aggregate claim: "all 4 below threshold" — no individual elements named
        let trace = make_trace(
            "coverage ≥50% for all 4 corpora — all 4 below threshold",
            &[],
        );
        let result = std::panic::catch_unwind(|| {
            assert_response_contains_per_element_enumeration(
                &trace,
                &["mika primary", "mika-skills", "mika-platform", "mika-cloud"],
            );
        });
        assert!(
            result.is_err(),
            "Should panic on aggregate claim without per-element enumeration"
        );
    }

    // --- Absence claim grounding tests ---

    #[test]
    fn absence_claim_grounded_passes_with_evidence() {
        let trace = make_trace(
            "searched for \"## R5 — Rollback procedure\" — not present in \
             PR body sections: Summary, Test plan, Breaking changes, Migration steps",
            &[],
        );
        assert_absence_claim_grounded(&trace, "R5");
    }

    #[test]
    fn absence_claim_grounded_passes_when_no_absence_claim() {
        // No absence keywords — helper should pass silently
        let trace = make_trace("All sections verified and correct.", &[]);
        assert_absence_claim_grounded(&trace, "R5");
    }

    #[test]
    fn absence_claim_grounded_fails_without_heading() {
        let trace = make_trace("section missing from the PR body", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_absence_claim_grounded(&trace, "R5");
        });
        assert!(
            result.is_err(),
            "Should panic when absence claim lacks searched heading"
        );
    }

    #[test]
    fn absence_claim_grounded_fails_without_evidence_list() {
        let trace = make_trace("R5 section missing from the PR body", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_absence_claim_grounded(&trace, "R5");
        });
        assert!(
            result.is_err(),
            "Should panic when absence claim lacks evidence list"
        );
    }

    // --- Per-line verification qualification tests ---

    #[test]
    fn per_line_verification_qualification_passes_with_correct_tiers() {
        let trace = make_trace(
            "Tarif: 25 € [vérifié: page CNI officielle service-public.fr]\n\
             Date de dernière mise à jour: 2024-01-15 [non vérifié — snippets uniquement, \
             je ne peux pas confirmer sans ouvrir la page officielle]",
            &[],
        );
        assert_per_line_verification_qualification(
            &trace,
            &[
                ("25 €", VerificationTier::Verified("page CNI officielle")),
                (
                    "Date de dernière mise à jour",
                    VerificationTier::SnippetOnly,
                ),
            ],
        );
    }

    #[test]
    fn per_line_verification_qualification_fails_when_element_missing() {
        let trace = make_trace("Tarif: 25 € [vérifié: page CNI officielle]", &[]);
        let result = std::panic::catch_unwind(|| {
            assert_per_line_verification_qualification(
                &trace,
                &[
                    ("25 €", VerificationTier::Verified("page CNI officielle")),
                    (
                        "Date de dernière mise à jour",
                        VerificationTier::SnippetOnly,
                    ),
                ],
            );
        });
        assert!(
            result.is_err(),
            "Should panic when a declared element is absent from the response"
        );
    }

    #[test]
    fn per_line_verification_qualification_fails_when_snippet_only_tagged_verified() {
        // Anti-pattern: element declared SnippetOnly but response tags it as [vérifié: ...].
        let trace = make_trace(
            "Tarif: 25 € [vérifié: page CNI officielle]\n\
             Date de dernière mise à jour: 2024-01-15 [vérifié: source secondaire]",
            &[],
        );
        let result = std::panic::catch_unwind(|| {
            assert_per_line_verification_qualification(
                &trace,
                &[
                    ("25 €", VerificationTier::Verified("page CNI officielle")),
                    (
                        "Date de dernière mise à jour",
                        VerificationTier::SnippetOnly,
                    ),
                ],
            );
        });
        assert!(
            result.is_err(),
            "Should panic when SnippetOnly element carries a [vérifié: ...] tag \
             (merged-verified-and-inferred anti-pattern)"
        );
    }

    #[test]
    fn per_line_verification_qualification_fails_when_no_tag_present() {
        // Element is named but no bracketed qualification tag appears in the window.
        let trace = make_trace(
            "Tarif: 25 €. Date de dernière mise à jour: 2024-01-15.",
            &[],
        );
        let result = std::panic::catch_unwind(|| {
            assert_per_line_verification_qualification(
                &trace,
                &[
                    ("25 €", VerificationTier::Verified("page CNI officielle")),
                    (
                        "Date de dernière mise à jour",
                        VerificationTier::SnippetOnly,
                    ),
                ],
            );
        });
        assert!(
            result.is_err(),
            "Should panic when elements are named without per-element qualification tags"
        );
    }
}
