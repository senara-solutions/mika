//! Golden dataset: end-to-end quality testing with real LLM providers.
//!
//! 25 curated scenarios across 4 capability classes:
//! - **Memory** (8): store/recall, disambiguation, time-based, updates, privacy
//! - **Tool selection** (8): calendar, memory search, GitHub, messaging, conflicts, multi-step
//! - **Conversation quality** (5): follow-up context, uncertainty, conciseness, rewind, compaction
//! - **Skill-specific** (4): self-dev plan coherence, qa-review, google-workspace, run_gh formatting
//!
//! Each scenario runs in three tiers (D6):
//! - **Unit** — MockLlmProvider, on every CI push
//! - **Integration** — real provider via `MIKA_EVAL_REAL_PROVIDERS` + `#[ignore]`
//! - **Calibration** — integration + `MIKA_EVAL_CALIBRATE=1` artifact capture
//!
//! See `README.md` in this directory for author-facing guidance.
//!
//! Reference: mika#339, `docs/plans/339-golden-dataset.md`

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// Re-export common test dependencies for scenario files
pub use mika_common::llm::mock::*;
pub use serde_json::json;

pub use super::assertions::*;
pub use super::harness::EvalHarness;

// --- Scenario modules (one per scenario, D2) ---

// Memory scenarios (8)
pub mod memory_category_routing;
pub mod memory_cross_session_persistence;
pub mod memory_fact_update;
pub mod memory_multi_fact_disambiguation;
pub mod memory_privacy_boundary;
pub mod memory_recall_single_fact;
pub mod memory_recall_variations;
pub mod memory_time_based_retrieval;

// Tool selection scenarios (8)
pub mod tool_selection_calendar_create;
pub mod tool_selection_conflict_two_plausible;
pub mod tool_selection_github_pr;
pub mod tool_selection_memory_search;
pub mod tool_selection_multi_step_sequence;
pub mod tool_selection_multi_turn_planning;
pub mod tool_selection_no_hallucinated_tools;
pub mod tool_selection_send_message;

// Conversation quality scenarios (5)
pub mod conversation_quality_admit_uncertainty;
pub mod conversation_quality_compaction_survival;
pub mod conversation_quality_concise_response;
pub mod conversation_quality_followup_context;
pub mod conversation_quality_rewind_semantics;

// Skill-specific scenarios (4)
pub mod skill_google_workspace_calendar;
pub mod skill_qa_review_bug_catch;
pub mod skill_run_gh_issue_creation;
pub mod skill_self_dev_plan_coherence;

// ─── Scenario classification (D1) ───

/// Capability class for scenario distribution tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScenarioClass {
    Memory,
    ToolSelection,
    ConversationQuality,
    SkillSpecific,
}

impl std::fmt::Display for ScenarioClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory => write!(f, "memory"),
            Self::ToolSelection => write!(f, "tool_selection"),
            Self::ConversationQuality => write!(f, "conversation_quality"),
            Self::SkillSpecific => write!(f, "skill_specific"),
        }
    }
}

// ─── Scenario metadata and registration (D7) ───

/// Per-scenario metadata registered at init time.
#[derive(Debug, Clone)]
pub struct GoldenScenarioMeta {
    /// Capability class this scenario belongs to.
    pub class: ScenarioClass,
    /// Expected token count for cost observability.
    /// Mismatches >2× flag in CI log output (D7).
    pub expected_tokens: u32,
    /// Human-readable description.
    pub description: &'static str,
}

/// Registry of golden scenarios with compile-time-equivalent uniqueness guard (D7).
///
/// Uses `HashMap::insert` — if `Some` is returned (duplicate), panics at init time:
/// "Duplicate scenario name '<name>' — did you copy-paste without renaming?"
pub struct GoldenRegistry {
    scenarios: Mutex<HashMap<&'static str, GoldenScenarioMeta>>,
}

impl GoldenRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            scenarios: Mutex::new(HashMap::new()),
        }
    }

    /// Register a scenario. Panics on duplicate name (D7 uniqueness guard).
    pub fn register(&self, name: &'static str, meta: GoldenScenarioMeta) {
        let mut map = self.scenarios.lock().unwrap();
        if let Some(existing) = map.insert(name, meta) {
            panic!(
                "Duplicate scenario name '{}' (class: {}) — did you copy-paste without renaming?",
                name, existing.class
            );
        }
    }

    /// Get all registered scenarios.
    pub fn scenarios(&self) -> HashMap<&'static str, GoldenScenarioMeta> {
        self.scenarios.lock().unwrap().clone()
    }

    /// Get the count of registered scenarios.
    pub fn len(&self) -> usize {
        self.scenarios.lock().unwrap().len()
    }

    /// Get the distribution of scenarios by class.
    pub fn class_distribution(&self) -> HashMap<ScenarioClass, usize> {
        let map = self.scenarios.lock().unwrap();
        let mut dist = HashMap::new();
        for meta in map.values() {
            *dist.entry(meta.class).or_insert(0) += 1;
        }
        dist
    }
}

/// Build the default golden registry with all 25 scenarios registered.
///
/// This is called once at test binary load time. The uniqueness guard
/// fires immediately on duplicate names — before any scenario runs.
pub fn default_golden_registry() -> GoldenRegistry {
    let registry = GoldenRegistry::new();

    // Memory (8)
    memory_recall_single_fact::register(&registry);
    memory_recall_variations::register(&registry);
    memory_multi_fact_disambiguation::register(&registry);
    memory_time_based_retrieval::register(&registry);
    memory_fact_update::register(&registry);
    memory_cross_session_persistence::register(&registry);
    memory_privacy_boundary::register(&registry);
    memory_category_routing::register(&registry);

    // Tool selection (8)
    tool_selection_calendar_create::register(&registry);
    tool_selection_memory_search::register(&registry);
    tool_selection_github_pr::register(&registry);
    tool_selection_send_message::register(&registry);
    tool_selection_conflict_two_plausible::register(&registry);
    tool_selection_no_hallucinated_tools::register(&registry);
    tool_selection_multi_step_sequence::register(&registry);
    tool_selection_multi_turn_planning::register(&registry);

    // Conversation quality (5)
    conversation_quality_followup_context::register(&registry);
    conversation_quality_admit_uncertainty::register(&registry);
    conversation_quality_concise_response::register(&registry);
    conversation_quality_rewind_semantics::register(&registry);
    conversation_quality_compaction_survival::register(&registry);

    // Skill-specific (4)
    skill_self_dev_plan_coherence::register(&registry);
    skill_qa_review_bug_catch::register(&registry);
    skill_google_workspace_calendar::register(&registry);
    skill_run_gh_issue_creation::register(&registry);

    registry
}

// ─── Golden scoring framework (D4) ───

/// Outcome of a golden scenario with hard assertions and soft tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenOutcome {
    /// Scenario name.
    pub scenario: String,
    /// Provider used.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Hard assertion results — every scenario MUST have ≥1.
    pub hard_assertions: Vec<HardAssertion>,
    /// Soft-tag judge output — quality:* namespace (D4).
    pub soft_tags: Vec<SoftTag>,
    /// Measured token count (input + output).
    pub tokens_measured: u64,
    /// Expected token count from scenario metadata.
    pub tokens_expected: u32,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Whether all hard assertions passed.
    pub passed: bool,
    /// Error message if the scenario failed to run at all.
    pub error: Option<String>,
}

/// Input params for constructing a `GoldenOutcome` (avoids clippy too-many-args).
pub struct GoldenOutcomeParams {
    pub scenario: String,
    pub provider: String,
    pub model: String,
    pub hard_assertions: Vec<HardAssertion>,
    pub soft_tags: Vec<SoftTag>,
    pub tokens_measured: u64,
    pub tokens_expected: u32,
    pub duration_ms: u64,
}

impl GoldenOutcome {
    /// Create an outcome from assertion results.
    ///
    /// # Panics
    /// Panics if `hard_assertions` is empty — every scenario MUST have ≥1 hard assertion.
    pub fn from_params(params: GoldenOutcomeParams) -> Self {
        assert!(
            !params.hard_assertions.is_empty(),
            "GoldenOutcome requires at least one hard assertion (scenario: {})",
            params.scenario
        );
        let passed = params.hard_assertions.iter().all(|a| a.passed);
        Self {
            scenario: params.scenario,
            provider: params.provider,
            model: params.model,
            hard_assertions: params.hard_assertions,
            soft_tags: params.soft_tags,
            tokens_measured: params.tokens_measured,
            tokens_expected: params.tokens_expected,
            duration_ms: params.duration_ms,
            passed,
            error: None,
        }
    }

    /// Create an error outcome when the scenario failed to execute.
    pub fn error(scenario: &str, provider: &str, model: &str, error: String) -> Self {
        Self {
            scenario: scenario.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            hard_assertions: Vec::new(),
            soft_tags: Vec::new(),
            tokens_measured: 0,
            tokens_expected: 0,
            duration_ms: 0,
            passed: false,
            error: Some(error),
        }
    }
}

/// A hard assertion — pass/fail, regression-gating (D4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardAssertion {
    /// Descriptive name of the assertion.
    pub name: String,
    /// Whether the assertion passed.
    pub passed: bool,
    /// Failure message (only populated when `passed == false`).
    pub message: Option<String>,
}

impl HardAssertion {
    pub fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: None,
        }
    }

    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: Some(message.into()),
        }
    }
}

/// A soft tag — LLM-judged quality signal, NOT regression-gating (D4).
///
/// Tags use the `quality:*` namespace (D4 vocabulary namespacing).
/// Sibling tickets own their own namespaces:
/// - #339: `quality:*` (concise, uncertain, actionable, verbose, off-topic)
/// - #740: `self-knowledge:*`
/// - #741: `grounding:*`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoftTag {
    /// Tag name in `quality:*` namespace.
    pub tag: String,
    /// Whether the tag was detected.
    pub present: bool,
}

impl SoftTag {
    pub fn new(tag: impl Into<String>, present: bool) -> Self {
        Self {
            tag: tag.into(),
            present,
        }
    }
}

/// Judge-tag vocabulary for #339 golden scenarios (D4).
///
/// Tags are constrained to this set — no free-form 0-10 scores.
pub const QUALITY_TAGS: &[&str] = &[
    "quality:concise",
    "quality:uncertain",
    "quality:actionable",
    "quality:verbose",
    "quality:off-topic",
];

/// Judge model pinned for baseline stability (D4).
/// Override via `MIKA_EVAL_JUDGE_MODEL` env var for offline developers.
pub const DEFAULT_JUDGE_MODEL: &str = "claude-sonnet-4-6";

/// Evaluate soft tags on a response using simple keyword heuristics.
///
/// For unit tests (mock provider), we use deterministic keyword matching.
/// Full LLM-judge evaluation is deferred to integration/calibration tiers.
pub fn evaluate_soft_tags_heuristic(response_text: &str) -> Vec<SoftTag> {
    let text = response_text.to_lowercase();
    let word_count = response_text.split_whitespace().count();

    vec![
        SoftTag::new("quality:concise", word_count < 100),
        SoftTag::new(
            "quality:verbose",
            word_count > 200 || text.contains("furthermore") || text.contains("additionally"),
        ),
        SoftTag::new(
            "quality:uncertain",
            text.contains("i'm not sure")
                || text.contains("i don't know")
                || text.contains("uncertain")
                || text.contains("might be"),
        ),
        SoftTag::new(
            "quality:actionable",
            text.contains("you should")
                || text.contains("you can")
                || text.contains("try")
                || text.contains("recommend"),
        ),
        SoftTag::new(
            "quality:off-topic",
            false, // Heuristic: assume on-topic for mock responses
        ),
    ]
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_uniqueness_guard() {
        let registry = GoldenRegistry::new();
        registry.register(
            "test_scenario",
            GoldenScenarioMeta {
                class: ScenarioClass::Memory,
                expected_tokens: 2000,
                description: "Test",
            },
        );

        let result = std::panic::catch_unwind(|| {
            registry.register(
                "test_scenario",
                GoldenScenarioMeta {
                    class: ScenarioClass::Memory,
                    expected_tokens: 2000,
                    description: "Duplicate",
                },
            );
        });
        assert!(result.is_err(), "Should panic on duplicate scenario name");
    }

    #[test]
    fn test_default_registry_has_25_scenarios() {
        let registry = default_golden_registry();
        assert_eq!(
            registry.len(),
            25,
            "Golden dataset should have exactly 25 scenarios (D1)"
        );
    }

    #[test]
    fn test_default_registry_class_distribution() {
        let registry = default_golden_registry();
        let dist = registry.class_distribution();
        assert_eq!(
            dist.get(&ScenarioClass::Memory).copied().unwrap_or(0),
            8,
            "Memory class should have 8 scenarios"
        );
        assert_eq!(
            dist.get(&ScenarioClass::ToolSelection)
                .copied()
                .unwrap_or(0),
            8,
            "ToolSelection class should have 8 scenarios"
        );
        assert_eq!(
            dist.get(&ScenarioClass::ConversationQuality)
                .copied()
                .unwrap_or(0),
            5,
            "ConversationQuality class should have 5 scenarios"
        );
        assert_eq!(
            dist.get(&ScenarioClass::SkillSpecific)
                .copied()
                .unwrap_or(0),
            4,
            "SkillSpecific class should have 4 scenarios"
        );
    }

    #[test]
    fn test_golden_outcome_from_params() {
        let outcome = GoldenOutcome::from_params(GoldenOutcomeParams {
            scenario: "test".to_string(),
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
            hard_assertions: vec![HardAssertion::pass("check1"), HardAssertion::pass("check2")],
            soft_tags: vec![SoftTag::new("quality:concise", true)],
            tokens_measured: 150,
            tokens_expected: 2000,
            duration_ms: 100,
        });
        assert!(outcome.passed);
        assert_eq!(outcome.hard_assertions.len(), 2);
        assert_eq!(outcome.soft_tags.len(), 1);
    }

    #[test]
    fn test_golden_outcome_fails_on_any_hard_failure() {
        let outcome = GoldenOutcome::from_params(GoldenOutcomeParams {
            scenario: "test".to_string(),
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
            hard_assertions: vec![
                HardAssertion::pass("check1"),
                HardAssertion::fail("check2", "wrong tool"),
            ],
            soft_tags: vec![],
            tokens_measured: 150,
            tokens_expected: 2000,
            duration_ms: 100,
        });
        assert!(!outcome.passed);
    }

    #[test]
    fn test_soft_tags_heuristic() {
        let tags = evaluate_soft_tags_heuristic("I'm not sure about that, but you should try X.");
        let uncertain = tags.iter().find(|t| t.tag == "quality:uncertain").unwrap();
        assert!(uncertain.present);
        let actionable = tags.iter().find(|t| t.tag == "quality:actionable").unwrap();
        assert!(actionable.present);
    }
}
