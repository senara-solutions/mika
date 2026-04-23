//! Scenario 3: `current_priorities` core memory drift (mika#732 class)
//!
//! Context: the agent has a `current_priorities` section in core memory with a known
//! ordered list. When asked about priorities, it should reproduce the exact order
//! from core memory, not a reordered or fabricated version.
//!
//! ## Hard Assertions
//! - Response MUST contain all 3 items in the exact order from core memory.
//! - Agent MUST either use injected context (core memory is always in the system
//!   prompt) or make a verified read.
//!
//! ## Tags
//! - `grounding:source-cited-correctly` — agent used core memory state faithfully
//!
//! ## Frozen Fixture
//! - `fixtures/current_priorities_drift_pre_fix.json` — pre-fix response with
//!   reversed priority order.
//!
//! Reference: mika#732, mika#741 D2 scenario 3

use super::*;

/// The known priority list that will be seeded into core memory.
const PRIORITIES: &[&str] = &[
    "Ship KG milestone #14 evaluation suite",
    "Fix Langfuse span tracking regression",
    "Review mika-cloud Helm chart for GPU nodepool",
];

/// Primary test: agent correctly reproduces priorities in the right order.
///
/// The agent's system prompt includes core memory, so the priorities are available
/// in context. The mock response faithfully reproduces them.
#[tokio::test]
async fn test_priorities_reproduced_in_correct_order() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Your current priorities are:\n\
                 1. Ship KG milestone #14 evaluation suite\n\
                 2. Fix Langfuse span tracking regression\n\
                 3. Review mika-cloud Helm chart for GPU nodepool",
        )])
        .build()
        .await?;

    // Seed core memory with the known priority list
    harness
        .db
        .with_db(|db| db.set_core_memory("mika", "current_priorities", &PRIORITIES.join("\n")))
        .await?;

    let trace = harness.run("What are my current priorities?").await?;

    // Hard: output exists
    assert_has_output(&trace);
    // Hard: priorities appear in the correct order
    grounding_assertions::assert_response_contains_in_order(&trace, PRIORITIES);
    // Agent should NOT need to call search_memory or read_agent_file —
    // core memory is in the system prompt. If it does call them, that's fine
    // (verified read), but it's not required.

    Ok(())
}

/// Regression-reproduction test: simulates pre-fix behavior where agent
/// reversed the priority order (responding from stale/imagined state).
#[tokio::test]
async fn test_regression_priorities_reversed_detected() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Pre-fix behavior: reversed order
            text_response(
                "Your current priorities are:\n\
                 1. Review mika-cloud Helm chart for GPU nodepool\n\
                 2. Fix Langfuse span tracking regression\n\
                 3. Ship KG milestone #14 evaluation suite",
            ),
        ])
        .build()
        .await?;

    // Seed core memory with the known priority list
    harness
        .db
        .with_db(|db| db.set_core_memory("mika", "current_priorities", &PRIORITIES.join("\n")))
        .await?;

    let trace = harness.run("What are my current priorities?").await?;

    // Verify the assertion framework catches the wrong order:
    // The pre-fix response has items in reversed order (3, 2, 1 instead of 1, 2, 3).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        grounding_assertions::assert_response_contains_in_order(&trace, PRIORITIES);
    }));
    assert!(
        result.is_err(),
        "Pre-fix regression: reversed priority order should be caught by \
         assert_response_contains_in_order"
    );

    Ok(())
}
