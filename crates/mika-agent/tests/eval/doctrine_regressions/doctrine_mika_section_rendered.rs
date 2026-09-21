//! Scenario: Mika Doctrine — prompt section rendered (mika#2292, AC1–AC7)
//!
//! Prompt-shape contract for the code-managed `## Mika Doctrine` section written
//! by `prompt::build_system_prompt` and `prompt::build_silent_prompt`.
//!
//! **Founding measurement (2026-09-11, champion tenant, canary Al).** Asked
//! « Qu'est-ce que la doctrine Mika ? », the agent answered "nothing found
//! called the Mika doctrine" **and then gave the philosophy anyway**, in the same
//! response. It was not a knowledge gap — it held every fragment of the answer
//! and had no name for it. Third occurrence of the class mika#1815 and mika#2290
//! each closed on another axis: a rule was already written for the shape
//! (rule 4, *uncertainty at t=0 and confidence at t=1 is confabulation*) and did
//! not apply, because the written scope of `## Self-Identity Discipline` was
//! "which model you are, which provider powers you, WHERE you run".
//!
//! ## What this scenario asserts, and what it deliberately does NOT
//!
//! It asserts the **shape of the prompt**, exactly like its sibling
//! `doctrine_prompt_section_rendered.rs`, and never the model's answer.
//! `MockLlmProvider` is sequential — it replays a scripted response — so a
//! scenario that "asked" it about the Mika doctrine would be verifying the
//! plumbing and calling it a behaviour. The behavioural half is the real-provider
//! replay in `doctrine_mika_answer_replayed.rs`, shipped **disarmed** with its
//! own reasoning (`#[ignore]` + `MIKA_EVAL_REAL_PROVIDERS`), and the part neither
//! covers is declared as manual verification (Sonde 1 of the plan).
//!
//! ## Hard assertions
//! - **AC1** — the heading is served by both non-compact builders, in both
//!   persona registers, and the aliases that failed to resolve are named in it.
//! - **AC2** — the operator register carries the five stances **with their why**.
//! - **AC3** — the stop is present and topical.
//! - **AC4** — nothing claims `exportable`; no hosting claim is made here.
//! - **AC5** — the family register carries the substance with no infrastructure
//!   term, and the two registers are not the same text.
//! - **AC7** — the compact (MikaModel) carve-out is preserved.
//!
//! Reference: mika#2292. Related: mika#2290 (hosting is a posed fact),
//! mika#1814 (distribution doctrine), mika#1783 (family persona has no maker it
//! knows about), mika#1925 (compact carve-out follow-up).

use chrono::{TimeZone, Utc};
use mika_agent::prompt::{
    self, ContextIdentityConfig, Identity, KgIdentityConfig, MIKA_DOCTRINE_BODY_FAMILY,
    MIKA_DOCTRINE_BODY_OPERATOR, MIKA_DOCTRINE_HEADING, PromptContext, SessionIdentityConfig,
    SkillsIdentityConfig, ToolsIdentityConfig,
};
use mika_common::home::{Deployment, PersonaProfile};

fn make_identity() -> Identity {
    Identity {
        name: "Mika".to_string(),
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

fn make_ctx<'a>(identity: &'a Identity, persona: PersonaProfile) -> PromptContext<'a> {
    PromptContext {
        soul_content: "",
        identity,
        core_memory: &[],
        is_onboarding: false,
        current_utc: Utc.with_ymd_and_hms(2026, 9, 18, 12, 0, 0).unwrap(),
        timezone: None,
        global_home_dir: None,
        channel_type: None,
        telegram_configured: false,
        home_dir: None,
        callback_context: None,
        runtime_provider: "test-provider",
        runtime_model: "test-model",
        stopped_topics: &[],
        // Cloud, because that is what the measured tenant was.
        deployment: Deployment::Cloud,
        persona_profile: persona,
        tenant_language: None,
    }
}

/// AC1 — the section is served in both registers, and it **names the aliases**.
/// The alias list is the load-bearing half: the measured failure was a name
/// resolution, not a content gap, so a section that carried the philosophy
/// without the word "doctrine" would leave the defect exactly where it was.
#[test]
fn test_ac1_doctrine_section_rendered_in_both_registers() {
    let identity = make_identity();

    for persona in [PersonaProfile::Operator, PersonaProfile::Family] {
        let out = prompt::build_system_prompt(&make_ctx(&identity, persona));

        assert!(
            out.contains(MIKA_DOCTRINE_HEADING),
            "AC1: {persona:?} prompt must contain `{MIKA_DOCTRINE_HEADING}`"
        );
        let lowered = out.to_lowercase();
        for alias in ["doctrine", "philosophy", "stand for"] {
            assert!(
                lowered.contains(alias),
                "AC1: {persona:?} prompt must name the alias `{alias}` — the measured \
                 defect was that \"doctrine\" resolved to nothing"
            );
        }
    }
}

/// AC1 — rule 6 forbids the measured answer by name and points at the section.
#[test]
fn test_ac1_rule_six_forbids_the_measured_answer() {
    let identity = make_identity();
    let out = prompt::build_system_prompt(&make_ctx(&identity, PersonaProfile::Operator));

    let discipline = out
        .split_once("## Self-Identity Discipline")
        .expect("AC1: the discipline section must be present")
        .1;

    assert!(
        discipline.contains(MIKA_DOCTRINE_HEADING),
        "AC1: rule 6 must designate the doctrine section by its heading"
    );
    assert!(
        discipline.contains("found nothing of that name is NOT acceptable"),
        "AC1: rule 6 must forbid the measured answer literally"
    );
}

/// AC2 — the five stances are present **with their why**. The *why* is what
/// makes it a doctrine rather than a feature list, and it is the ticket's
/// explicit demand.
#[test]
fn test_ac2_operator_register_carries_each_stance_with_its_why() {
    let identity = make_identity();
    let out = prompt::build_system_prompt(&make_ctx(&identity, PersonaProfile::Operator));
    let lowered = out.to_lowercase();

    // The five stances.
    for stance in [
        "open source",
        "belongs to that person",
        "proactive",
        "you remember what matters",
        "grows by invitation",
    ] {
        assert!(
            lowered.contains(stance),
            "AC2: the operator register must carry the stance `{stance}`"
        );
    }

    // Their reasons — one probe per stance, on the clause that carries the why.
    for why in [
        "depend on a single company",
        "mental load",
        "reintroduce oneself",
    ] {
        assert!(
            lowered.contains(why),
            "AC2: the operator register must carry the reason `{why}` — a stance \
             without its why is a feature list"
        );
    }

    // Growth-by-invitation is **cited**, never re-narrated: the duplication that
    // drifts is the class `grooming_marker` (mika#2158) had to close.
    assert!(
        out.contains(prompt::DISTRIBUTION_DOCTRINE_HEADING),
        "AC2: the invitation stance must refer to its own section rather than \
         restating it"
    );
}

/// AC3 — the stop exists, and it is topical. Its *non*-enumerative property is
/// asserted structurally in `prompt::tests::mika2292_no_spiritual_referent_*`,
/// whose denylist lives under `#[cfg(test)]` so the list cannot itself become
/// the leak; here we assert only that a stop is served at all.
#[test]
fn test_ac3_spiritual_stop_is_present_and_topical() {
    let identity = make_identity();

    let operator = prompt::build_system_prompt(&make_ctx(&identity, PersonaProfile::Operator));
    assert!(
        operator.contains("You do not know them and you will not invent them"),
        "AC3: the operator register must carry the stop"
    );
    assert!(
        operator.contains("choice of Mika's creator"),
        "AC3: the operator register carries the bearing's formulation"
    );

    // Family register: same stop, **no creator referent** (mika#1783 — the being
    // does not have a maker it knows about). The asymmetry is a decision, D4.
    let family = prompt::build_system_prompt(&make_ctx(&identity, PersonaProfile::Family));
    assert!(
        family.contains("that you will not make up"),
        "AC3: the family register must carry the stop too"
    );
    assert!(
        !MIKA_DOCTRINE_BODY_FAMILY.to_lowercase().contains("creator"),
        "AC3/D4: the family stop must establish no creator referent"
    );
}

/// AC4 — verified facts only. `exportable` is claimed nowhere in what this
/// ticket adds, and the bodies make no hosting claim: that question is referred
/// to `## Runtime`, whose hosting line is its ground truth (mika#2290).
#[test]
fn test_ac4_no_unverified_claim_is_added() {
    for (register, body) in [
        ("operator", MIKA_DOCTRINE_BODY_OPERATOR),
        ("family", MIKA_DOCTRINE_BODY_FAMILY),
    ] {
        assert!(
            !body.to_lowercase().contains("exportable"),
            "AC4: the {register} body must not claim `exportable` — nothing in this \
             repository can verify it"
        );
        assert!(
            body.contains("## Runtime"),
            "AC4: the {register} body must refer the hosting question to `## Runtime` \
             rather than answering it"
        );
    }

    // MIT is claimed of the **engine**, never of Mika as a whole: the cloud
    // console is a separate closed codebase, so an unqualified claim would be
    // false.
    assert!(
        MIKA_DOCTRINE_BODY_OPERATOR.contains("The engine that runs you is open source"),
        "AC4: the MIT claim must be scoped to the engine"
    );
}

/// AC5 — two registers, one fact. The family register says the same substance
/// with no infrastructure term (`FAMILY_SOUL` forbids it « jamais, même si on te
/// le demande »), and the two bodies are genuinely different text.
#[test]
fn test_ac5_family_register_says_it_without_infrastructure_vocabulary() {
    let lowered = MIKA_DOCTRINE_BODY_FAMILY.to_lowercase();
    for forbidden in [
        "open source",
        "licence",
        "repository",
        "server",
        "self-host",
    ] {
        assert!(
            !lowered.contains(forbidden),
            "AC5: the family body must not carry `{forbidden}`"
        );
    }

    // Control: it still says the substance, rather than saying nothing.
    assert!(
        lowered.contains("belongs to the person"),
        "AC5: the family body must still carry the substance"
    );

    assert_ne!(
        MIKA_DOCTRINE_BODY_OPERATOR, MIKA_DOCTRINE_BODY_FAMILY,
        "AC5: the persona match must not fall through to one arm"
    );
}

/// AC1 — silent turns carry the section too. The material register is harmless
/// in silent mode; the **stop** is not — a silent turn that wrote a memory fact
/// on the spiritual register would poison every later turn through core memory,
/// which is re-injected into every prompt.
#[test]
fn test_ac1_silent_prompt_carries_the_doctrine_section() {
    let identity = make_identity();
    let ctx = prompt::SilentPromptContext {
        soul_content: "",
        identity: &identity,
        core_memory: &[],
        pending_commitments: &[],
        trigger_context: "heartbeat",
        current_utc: Utc.with_ymd_and_hms(2026, 9, 18, 12, 0, 0).unwrap(),
        timezone: None,
        telegram_configured: false,
        has_message_sender: true,
        recent_conversations: None,
        recent_audit_events: None,
        home_dir: None,
        task_health: None,
        stored_preferences: &[],
        stopped_topics: &[],
        runtime_provider: "test-provider",
        runtime_model: "test-model",
        deployment: Deployment::Cloud,
        persona_profile: PersonaProfile::Family,
        tenant_language: None,
    };

    let out = prompt::build_silent_prompt(&ctx);
    assert!(
        out.contains(MIKA_DOCTRINE_HEADING),
        "AC1: the silent prompt must render the doctrine section"
    );
    assert!(
        out.contains(MIKA_DOCTRINE_BODY_FAMILY),
        "AC1: the silent prompt must render the register's body"
    );
}

/// AC7 — the compact (MikaModel) carve-out is preserved, and pinned as a
/// **decision** rather than an oversight. Its cost is real and stated at the
/// point of omission: on that path the measured defect stays open, because the
/// ≤5 KB budget cannot afford the section and the measured population is not
/// served by it. Joined to mika#1925 with the three sibling carve-outs.
#[test]
fn test_ac7_compact_carve_out_preserved() {
    let identity = make_identity();
    let out = prompt::build_compact_system_prompt(&make_ctx(&identity, PersonaProfile::Operator));

    assert!(
        !out.contains(MIKA_DOCTRINE_HEADING),
        "AC7: the compact prompt must not render the doctrine section"
    );
    // Sanity: absence is the carve-out, not a broken builder.
    assert!(
        out.contains("## Identity"),
        "AC7 sanity: the compact builder still renders its other sections"
    );
    assert!(
        out.len() <= 5 * 1024,
        "AC7: the carve-out exists for the ≤5 KB budget; compact prompt is {} bytes",
        out.len()
    );
}

/// The section must precede the rule that cites it — **order, not adjacency**.
/// It is docked to `## Distribution Doctrine` rather than to `## Runtime` (the
/// two doctrines read together, and `## Runtime` is a block of machine facts
/// populated at execution time where this one is constant at the binary), so it
/// is not the discipline's immediate neighbour; the citation names it by heading
/// precisely so distance costs nothing.
#[test]
fn test_doctrine_precedes_the_discipline_that_cites_it() {
    let identity = make_identity();
    let out = prompt::build_system_prompt(&make_ctx(&identity, PersonaProfile::Operator));

    let distribution = out
        .find(prompt::DISTRIBUTION_DOCTRINE_HEADING)
        .expect("distribution doctrine must be present");
    let doctrine = out
        .find(MIKA_DOCTRINE_HEADING)
        .expect("mika doctrine must be present");
    let discipline = out
        .find("## Self-Identity Discipline")
        .expect("discipline must be present");

    assert!(
        distribution < doctrine && doctrine < discipline,
        "the cited section must precede the rule that cites it"
    );
}
