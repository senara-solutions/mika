//! Scenario: Distribution Doctrine — prompt section rendered (AC1 + AC9 + AC11)
//!
//! Prompt-shape contract for the code-managed `## Distribution Doctrine`
//! section written by `prompt::build_system_prompt` and
//! `prompt::build_silent_prompt` (mika#1814).
//!
//! These assertions are the headless-safe verification path for AC11:
//! mika-qa (running in CI) verifies the prompt template *cites* the canonical
//! bearing memory `project_mika_invitation_only_no_public_launch` by name;
//! the operator (Vincent) verifies the memory file exists at
//! `~/.claude/projects/-data-workspace-mika-platform/memory/…` before applying
//! the `ready` label. This test owns the mika-qa half of that enforcement
//! split — the operator half is not automatable from within CI.
//!
//! ## Hard Assertions
//! - **AC1a:** `build_system_prompt` output contains `## Distribution Doctrine`.
//! - **AC1b:** Output names at least {Show HN, Product Hunt, Reddit launch,
//!   Twitter promo, growth-hack} as prohibited surfaces.
//! - **AC1c:** Output carries both the French and English redirect-script
//!   fragments so a bilingual Mika lands the right language without a
//!   code-switch.
//! - **AC9:** `build_compact_system_prompt` output does NOT contain the
//!   heading — the MikaModel ≤5 KB budget carve-out is preserved.
//! - **AC11:** The prompt-template cites the bearing memory by name
//!   (`project_mika_invitation_only_no_public_launch`).
//!
//! Reference: mika#1814 AC1, AC9, AC11.

use chrono::{TimeZone, Utc};
use mika_agent::prompt::{
    self, ContextIdentityConfig, Identity, KgIdentityConfig, PromptContext, SessionIdentityConfig,
    SkillsIdentityConfig, ToolsIdentityConfig,
};

fn make_identity(name: &str) -> Identity {
    Identity {
        name: name.to_string(),
        emoji: "M".to_string(),
        reflection: None,
        heartbeat: None,
        kg: KgIdentityConfig::default(),
        skills: SkillsIdentityConfig::default(),
        tools: ToolsIdentityConfig::default(),
        context: ContextIdentityConfig::default(),
        session: SessionIdentityConfig::default(),
        curator: None,
    }
}

fn make_ctx<'a>(soul_content: &'a str, identity: &'a Identity) -> PromptContext<'a> {
    PromptContext {
        soul_content,
        identity,
        core_memory: &[],
        is_onboarding: false,
        current_utc: Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap(),
        timezone: None,
        global_home_dir: None,
        channel_type: None,
        telegram_configured: false,
        home_dir: None,
        callback_context: None,
        runtime_provider: "test-provider",
        runtime_model: "test-model",
        stopped_topics: &[],
    }
}

#[test]
fn test_ac1_distribution_doctrine_section_rendered_operator_tier() {
    // Operator-tier persona (English soul preamble stand-in).
    let identity = make_identity("Mika");
    let ctx = make_ctx(
        "# Mika — Executive Assistant\n\nSharp, proactive.",
        &identity,
    );
    let out = prompt::build_system_prompt(&ctx);

    assert!(
        out.contains(prompt::DISTRIBUTION_DOCTRINE_HEADING),
        "AC1a: operator-tier prompt must contain the Distribution Doctrine \
         heading. Prompt was:\n{}",
        out
    );
}

#[test]
fn test_ac1_distribution_doctrine_section_rendered_family_tier() {
    // Family-tier persona (French soul preamble stand-in).
    let identity = make_identity("Mika");
    let ctx = make_ctx("# Mika — Compagnon personnel (famille)", &identity);
    let out = prompt::build_system_prompt(&ctx);

    assert!(
        out.contains(prompt::DISTRIBUTION_DOCTRINE_HEADING),
        "AC1a: family-tier prompt must contain the Distribution Doctrine \
         heading — same code-managed section as operator tier."
    );
}

#[test]
fn test_ac1_distribution_doctrine_names_prohibited_surfaces() {
    let identity = make_identity("Mika");
    let ctx = make_ctx("", &identity);
    let out = prompt::build_system_prompt(&ctx);

    // AC1b: minimum required prohibited-surface list.
    for surface in [
        "Show HN",
        "Product Hunt",
        "Reddit launch",
        "Twitter",
        "growth-hack",
    ] {
        assert!(
            out.contains(surface),
            "AC1b: prompt must name '{surface}' as a prohibited public-launch \
             surface. Absent from prompt."
        );
    }
}

#[test]
fn test_ac1_distribution_doctrine_bilingual_redirect_script() {
    let identity = make_identity("Mika");
    let ctx = make_ctx("", &identity);
    let out = prompt::build_system_prompt(&ctx);

    // AC1c: French redirect fragment.
    assert!(
        out.contains("Mika grandit par invitation entre proches"),
        "AC1c: prompt must carry the French redirect script fragment"
    );
    // AC1c: English redirect fragment.
    assert!(
        out.contains("Mika grows through personal invitation"),
        "AC1c: prompt must carry the English redirect script fragment"
    );
}

#[test]
fn test_ac11_bearing_memory_cited_by_name() {
    let identity = make_identity("Mika");
    let ctx = make_ctx("", &identity);
    let out = prompt::build_system_prompt(&ctx);

    // AC11 mika-qa half: prompt template cites the canonical bearing memory
    // name (the operator's institutional-memory file lives outside the PR's
    // changed-file set — mika-qa verifies the string reference, not the file).
    assert!(
        out.contains("project_mika_invitation_only_no_public_launch"),
        "AC11: prompt must cite the canonical bearing memory \
         `project_mika_invitation_only_no_public_launch` by name so the \
         operator-authored institutional-memory anchor is discoverable from \
         the prompt itself."
    );
}

#[test]
fn test_ac9_compact_provider_carve_out_preserves_budget() {
    let identity = make_identity("Mika");
    let ctx = make_ctx("Sharp, proactive.", &identity);
    let out = prompt::build_compact_system_prompt(&ctx);

    // AC9: compact prompt intentionally omits the doctrine section — the
    // ≤5 KB MikaModel budget cannot afford it. Follow-up: mika#1925 sibling
    // for a size-capped variant when MikaModel goes live for real tenants.
    assert!(
        !out.contains(prompt::DISTRIBUTION_DOCTRINE_HEADING),
        "AC9: compact-provider prompt MUST NOT render the Distribution \
         Doctrine section (MikaModel budget carve-out). Present in compact \
         prompt:\n{}",
        out
    );

    // Sanity check: the compact prompt still contains the identity heading —
    // absence of our section is not because the whole builder is broken.
    assert!(
        out.contains("## Identity"),
        "sanity: compact prompt should still render ## Identity heading"
    );
}
