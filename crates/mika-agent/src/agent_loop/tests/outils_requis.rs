//! `collect_required_tools` et `filter_available_required_tools`.
//!
//! Les contraintes de skill qui arment la porte des outils requis (#270,
//! #463), l'augmentation par ressource citée (#863), la branche citée qui
//! n'arme rien (mika#2755), l'effet de la restriction par tour (mika#2363),
//! la chaîne complète de l'AC (mika#2757) et les cinq cas du désarmement
//! avec leur raison (mika#2814).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- collect_required_tools tests (#270, #463) --

#[test]
fn test_collect_required_tools_empty_when_no_constraints() {
    let s1 = make_skill_entry("search", 30, &["web_search"]);
    let s2 = make_skill_entry("calc", 10, &["calculate"]);
    let matched = vec![
        MatchedSkill {
            entry: &s1,
            reason: MatchReason::Keyword,
        },
        MatchedSkill {
            entry: &s2,
            reason: MatchReason::Keyword,
        },
    ];
    let required = collect_required_tools(&matched, "");
    assert!(required.is_empty());
}

#[test]
fn test_collect_required_tools_union_across_keyword_matched_skills() {
    // Two keyword-matched skills with different required_tools — result is the union
    let s1 = make_skill_entry_with_constraints("qa-review", 30, &["run_gh"], &["run_gh"]);
    let s2 =
        make_skill_entry_with_constraints("code-check", 30, &["run_lint"], &["run_lint", "run_gh"]);
    let matched = vec![
        MatchedSkill {
            entry: &s1,
            reason: MatchReason::Keyword,
        },
        MatchedSkill {
            entry: &s2,
            reason: MatchReason::Keyword,
        },
    ];
    let required = collect_required_tools(&matched, "");
    assert_eq!(required.len(), 2);
    assert!(required.contains("run_gh"));
    assert!(required.contains("run_lint"));
}

#[test]
fn test_collect_required_tools_ignores_always_on_matched_skills() {
    // Skill matched via always_on should NOT contribute required_tools (#463)
    let s1 = make_skill_entry_with_constraints(
        "self-dev",
        30,
        &["run_claude_pilot"],
        &["run_claude_pilot"],
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    let required = collect_required_tools(&matched, "");
    assert!(
        required.is_empty(),
        "always_on skills should not enforce required_tools"
    );
}

#[test]
fn test_collect_required_tools_ignores_dependency_matched_skills() {
    // Skill pulled in as a dependency should NOT contribute required_tools
    let s1 = make_skill_entry_with_constraints(
        "dev-pilot",
        30,
        &["run_claude_pilot"],
        &["run_claude_pilot"],
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Dependency,
    }];
    let required = collect_required_tools(&matched, "");
    assert!(
        required.is_empty(),
        "dependency skills should not enforce required_tools"
    );
}

#[test]
fn test_collect_required_tools_keyword_match_on_always_on_skill_enforces() {
    // When an always_on skill is matched via keyword, its required_tools ARE enforced
    let s1 = make_skill_entry_with_constraints(
        "self-dev",
        30,
        &["run_claude_pilot"],
        &["run_claude_pilot"],
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    let required = collect_required_tools(&matched, "");
    assert_eq!(required.len(), 1);
    assert!(required.contains("run_claude_pilot"));
}

#[test]
fn test_collect_required_tools_mixed_reasons() {
    // Mix of keyword and always_on matches — only keyword contributes
    let s1 = make_skill_entry_with_constraints(
        "self-dev",
        30,
        &["run_claude_pilot"],
        &["run_claude_pilot"],
    );
    let s2 = make_skill_entry_with_constraints("qa-check", 30, &["run_tests"], &["run_tests"]);
    let matched = vec![
        MatchedSkill {
            entry: &s1,
            reason: MatchReason::AlwaysOn,
        },
        MatchedSkill {
            entry: &s2,
            reason: MatchReason::Keyword,
        },
    ];
    let required = collect_required_tools(&matched, "");
    assert_eq!(required.len(), 1);
    assert!(required.contains("run_tests"));
    assert!(!required.contains("run_claude_pilot"));
}

// -- mika#2755: une branche citée n'arme aucun required_tools --

/// Message du webhook fondateur de mika#2755, **mot pour mot**.
const MIKA2755_CHECK_SUITE_MESSAGE: &str = "[GitHub] Check suite success on senara-solutions/mika (branch: fix/2750/dispatch-un-groom-ready-label-qui-crashe)";

#[test]
fn mika2755_a_cited_branch_arms_no_required_tools() {
    // La scène du défaut, réduite à son os : un skill nommé `groom` —
    // `make_skill_entry_with_constraints` en fait son propre mot-clé — qui
    // exige `run_claude_pilot_groom`. Le slug de la branche contient
    // `groom` entre deux traits d'union, donc les deux ancres `\b` de
    // mika#1878 y sont satisfaites.
    let groom = make_skill_entry_with_constraints(
        "groom",
        30,
        &["run_claude_pilot_groom"],
        &["run_claude_pilot_groom"],
    );
    let skills = vec![groom];

    let matched = crate::skills::matcher::match_skills(&skills, MIKA2755_CHECK_SUITE_MESSAGE);
    let required = collect_required_tools(&matched, MIKA2755_CHECK_SUITE_MESSAGE);

    assert!(
        required.is_empty(),
        "un `check_suite` qui cite une branche ne doit armer aucun outil requis — \
             c'est ce `required_tools` qui a produit le re-prompt et la tâche fantôme \
             de mika#2755 ; obtenu : {required:?}"
    );
}

#[test]
fn mika2755_a_human_groom_request_still_arms_the_tool() {
    // Contre-épreuve : le chemin nominal reste entier. Sans elle, vider
    // `required_tools` pour tout le monde passerait aussi.
    let groom = make_skill_entry_with_constraints(
        "groom",
        30,
        &["run_claude_pilot_groom"],
        &["run_claude_pilot_groom"],
    );
    let skills = vec![groom];

    let matched = crate::skills::matcher::match_skills(&skills, "groom mika#2750");
    let required = collect_required_tools(&matched, "groom mika#2750");

    assert_eq!(required.len(), 1, "obtenu : {required:?}");
    assert!(required.contains("run_claude_pilot_groom"));
}

// -- mika#2363: what the per-turn skill restriction does to the output
//    contract (V4 / B2) --

/// The three mika-arch skills as the registry carries them: all `always_on`,
/// each declaring its own pass's suffix lines.
fn arch_skill(name: &str, suffix_lines: &[&str]) -> SkillEntry {
    let mut entry = make_skill_entry(name, 30, &[]);
    entry.manifest.skill.always_on = true;
    entry.manifest.output.required_suffix_lines =
        suffix_lines.iter().map(|s| s.to_string()).collect();
    entry
}

fn groom_ticket_skill() -> SkillEntry {
    arch_skill(
        "mika-arch-groom-ticket",
        &[
            "Disposition: READY",
            "Disposition: ITERATE",
            "Disposition: ESCALATE",
        ],
    )
}

fn second_review_skill() -> SkillEntry {
    arch_skill(
        "mika-arch-second-review",
        &["Verdict: GROOMED", "Verdict: ESCALATE"],
    )
}

#[test]
fn mika2363_unrestricted_arch_turn_accepts_both_verdict_families() {
    // The pre-mika#2363 state, asserted so the narrowing below is a measured
    // change and not a coincidence. A first-pass turn could satisfy the
    // suffix-line guard by emitting `Verdict: GROOMED` — a contract
    // `_parse_disposition` does not read, which surfaces as UNPARSED.
    let a = groom_ticket_skill();
    let b = second_review_skill();
    let matched = vec![
        MatchedSkill {
            entry: &a,
            reason: MatchReason::AlwaysOn,
        },
        MatchedSkill {
            entry: &b,
            reason: MatchReason::AlwaysOn,
        },
    ];

    let accepted = collect_required_suffix_lines(&matched);
    assert_eq!(accepted.len(), 5);
    assert!(accepted.contains(&"Verdict: GROOMED".to_string()));
    assert!(accepted.contains(&"Disposition: READY".to_string()));
}

#[test]
fn mika2363_a_restricted_turn_accepts_only_its_own_pass_contract() {
    // After the restriction the matched set is the single declared pass —
    // `skills::tests::mika2363_only_skills_narrows_the_matched_set_to_one`
    // asserts that half. This is the consequence on the output contract: the
    // accept-set is the running pass's, and the sister family is gone.
    //
    // B2 calls this a **tightening**, and it is the one behaviour change to
    // watch post-deploy: a model that used to satisfy the guard with the
    // wrong family now costs one corrective re-prompt instead of passing.
    let a = groom_ticket_skill();
    let matched = vec![MatchedSkill {
        entry: &a,
        reason: MatchReason::AlwaysOn,
    }];

    let accepted = collect_required_suffix_lines(&matched);
    assert_eq!(
        accepted,
        vec![
            "Disposition: READY".to_string(),
            "Disposition: ITERATE".to_string(),
            "Disposition: ESCALATE".to_string(),
        ]
    );
    assert!(
        !accepted.iter().any(|l| l.starts_with("Verdict:")),
        "the second-pass contract must not be accepted on a first-pass turn"
    );
}

#[test]
fn mika2363_escalation_still_has_a_declared_target_under_restriction() {
    // `escalate_unattested_disposition` withdraws a non-terminal disposition
    // into the ESCALATE *of its own family*, and refuses when that line is
    // not declared. Narrowing the accept-set could have removed the target
    // and silently disarmed the mika#2037 fail-visible path, so both
    // restricted shapes are checked here rather than assumed.
    for (entry, withdrawn, expected) in [
        (
            groom_ticket_skill(),
            "Disposition: READY",
            "Disposition: ESCALATE",
        ),
        (
            second_review_skill(),
            "Verdict: GROOMED",
            "Verdict: ESCALATE",
        ),
    ] {
        let matched = vec![MatchedSkill {
            entry: &entry,
            reason: MatchReason::AlwaysOn,
        }];
        let accepted = collect_required_suffix_lines(&matched);
        assert!(
            accepted.contains(&expected.to_string()),
            "{withdrawn} must still have a declared ESCALATE target after restriction"
        );
    }
}

// -- collect_required_tools pre-fetch augmentation tests (#863) --

/// Like `make_skill_entry_with_constraints` but also sets the pre-fetch opt-in flag.
fn make_skill_entry_with_pre_fetch(
    name: &str,
    timeout: u64,
    tool_names: &[&str],
    required_tools: &[&str],
    pre_fetch: bool,
) -> SkillEntry {
    let mut entry = make_skill_entry_with_constraints(name, timeout, tool_names, required_tools);
    entry
        .manifest
        .constraints
        .required_fetches_for_quoted_resources = pre_fetch;
    entry
}

#[test]
fn test_collect_required_tools_pre_fetch_augments_on_quoted_issue() {
    // Skill with pre-fetch opt-in, user message contains quoted issue body
    let s1 = make_skill_entry_with_pre_fetch("arch-review", 30, &["gh_read"], &["gh_read"], true);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    let msg = "Review this plan:\n\nissue/788\n```\nThe issue body here.\n```\n";
    let required = collect_required_tools(&matched, msg);
    assert!(
        required.contains("gh_read"),
        "gh_read should be in required set (static + augmented)"
    );
}

#[test]
fn test_collect_required_tools_pre_fetch_no_augment_on_plain_prose() {
    // Skill with pre-fetch opt-in, but user message has no fenced content
    let s1 = make_skill_entry_with_pre_fetch("arch-review", 30, &["gh_read"], &["gh_read"], true);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    let msg = "Review plan for #788. Check issue #654 too.";
    let required = collect_required_tools(&matched, msg);
    // gh_read is still in required set from static required_tools
    assert!(required.contains("gh_read"));
    // But the augmentation didn't add anything additional
    assert_eq!(required.len(), 1);
}

#[test]
fn test_collect_required_tools_pre_fetch_skipped_for_always_on() {
    // AlwaysOn skill with pre-fetch opt-in should NOT trigger augmentation
    let s1 = make_skill_entry_with_pre_fetch("arch-review", 30, &["gh_read"], &["gh_read"], true);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    let msg = "Review this:\n\nissue/788\n```\nThe issue body.\n```\n";
    let required = collect_required_tools(&matched, msg);
    // AlwaysOn doesn't contribute any required_tools at all (#463)
    assert!(
        required.is_empty(),
        "AlwaysOn skills should not enforce required_tools or pre-fetch"
    );
}

#[test]
fn test_collect_required_tools_pre_fetch_disabled_by_default() {
    // Skill WITHOUT pre-fetch opt-in — quoted resources do not augment
    let s1 = make_skill_entry_with_pre_fetch("qa-review", 30, &["run_gh"], &["run_gh"], false);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    let msg = "Review this:\n\nissue/788\n```\nThe issue body.\n```\n";
    let required = collect_required_tools(&matched, msg);
    // Only static required_tools, no augmentation
    assert_eq!(required.len(), 1);
    assert!(required.contains("run_gh"));
}

// -- filter_available_required_tools tests (#516, #517) --

// `make_resolved_skill_tool`, `make_registry_with_tool` et `served` sont promus
// dans `tests/mod.rs` : le frère `pr_review_groom` les nomme par
// `super::super::tests::…`, donc le chemin doit rester celui du module parent.

#[test]
fn test_filter_available_required_tools_keeps_builtin() {
    let tools = make_registry_with_tool("run_gh");
    let required: HashSet<String> = ["run_gh".to_string()].into();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let filtered = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_gh"]),
        "",
        None,
    );
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains("run_gh"));
}

#[test]
fn test_filter_available_required_tools_removes_unavailable() {
    let tools = ToolRegistry::new();
    let required: HashSet<String> = ["run_shell".to_string(), "nonexistent".to_string()].into();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let filtered = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_shell", "nonexistent"]),
        "",
        None,
    );
    assert!(
        filtered.is_empty(),
        "unavailable tools should be filtered out"
    );
}

#[test]
fn test_filter_available_required_tools_keeps_skill_tools() {
    let tools = ToolRegistry::new();
    let skill_tool = make_resolved_skill_tool("qa_pr_review");
    let mut skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    skill_map.insert("qa_pr_review".to_string(), &skill_tool);

    let required: HashSet<String> = ["qa_pr_review".to_string()].into();
    let filtered = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["qa_pr_review"]),
        "",
        None,
    );
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains("qa_pr_review"));
}

// -- mika#2757 — le test de l'AC : la chaîne complète --

// `TICKET_2750_CLOSED_MSG` est promu dans `tests/mod.rs` avec son
// doc-comment, pour la même raison que les trois helpers ci-dessus.

/// mika#2757 — l'AC, de bout en bout :
/// `match_skills` → `collect_required_tools` → `filter_available_required_tools` → `∅`.
///
/// La **première** assertion est ce qui donne sa valeur à la seconde : elle
/// situe le correctif. Le mot-clé s'arme toujours — ce vol désarme en aval,
/// il n'empêche pas de matcher, et ne touche donc pas au prompt que la skill
/// injecte.
#[test]
fn mika2757_un_tour_fallthrough_citant_groom_narme_aucune_exigence() {
    // `make_skill_entry_with_constraints` pose `keywords = [name]`, donc une
    // skill nommée « groom » reproduit le mot-clé de dev-groom sans ajouter
    // un builder.
    let skills = vec![make_skill_entry_with_constraints(
        "groom",
        30,
        &[],
        &["run_claude_pilot_groom"],
    )];

    let matched = crate::skills::matcher::match_skills(&skills, TICKET_2750_CLOSED_MSG);
    let required = collect_required_tools(&matched, TICKET_2750_CLOSED_MSG);
    assert_eq!(
        required,
        HashSet::from(["run_claude_pilot_groom".to_string()]),
        "le mot-clé doit toujours s'armer : le vol désarme, il n'empêche pas de matcher"
    );

    // L'outil existe et il est servi — la contradiction n'est donc pas une
    // absence d'outil, c'est le tour qui lui interdit de réussir.
    let tools = make_registry_with_tool("run_claude_pilot_groom");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_claude_pilot_groom"]),
        TICKET_2750_CLOSED_MSG,
        None,
    );

    assert!(
        effective.is_empty(),
        "un tour qui interdit le dispatch ne peut pas exiger un outil de dispatch — \
             exigence effective restée : {effective:?}"
    );
}

/// Le contrôle négatif humain de l'AC. Sans lui, le test ci-dessus passerait
/// aussi sur un désarmement qui désarme tout.
#[test]
fn mika2757_une_demande_humaine_de_grooming_arme_toujours_loutil() {
    const HUMAN_MSG: &str = "groom mika#2750";

    let skills = vec![make_skill_entry_with_constraints(
        "groom",
        30,
        &[],
        &["run_claude_pilot_groom"],
    )];

    let matched = crate::skills::matcher::match_skills(&skills, HUMAN_MSG);
    let required = collect_required_tools(&matched, HUMAN_MSG);
    assert_eq!(
        required,
        HashSet::from(["run_claude_pilot_groom".to_string()])
    );

    let tools = make_registry_with_tool("run_claude_pilot_groom");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_claude_pilot_groom"]),
        HUMAN_MSG,
        None,
    );

    assert_eq!(
        effective,
        HashSet::from(["run_claude_pilot_groom".to_string()]),
        "une vraie demande humaine doit garder son exigence intacte"
    );
}

#[test]
fn test_filter_available_required_tools_mixed() {
    // Mix of available (builtin) and unavailable tools — only available kept
    let tools = make_registry_with_tool("run_gh");
    let required: HashSet<String> = ["run_gh".to_string(), "run_shell".to_string()].into();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let filtered = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_gh", "run_shell"]),
        "",
        None,
    );
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains("run_gh"));
    assert!(
        !filtered.contains("run_shell"),
        "unavailable run_shell should be filtered out"
    );
}

// -- mika#2757 — les cinq cas du désarmement, au site du désarmement --
//
// D6 (ci-dessus) mesure la chaîne entière, donc il ne peut pas dire *quelle*
// branche a retiré l'outil. Ces cinq cas interrogent la fonction seule, et
// c'est là que se lit la frontière : les deux premiers désarment, les trois
// derniers ne désarment pas, et chacun de ces trois-là est une raison
// différente de ne pas désarmer — un outil qui n'est pas de dispatch, un tour
// qui n'est pas dans le domaine, un tour où le dispatch est l'ordre même.

/// Cas 1 — la forme mesurée : un outil de dispatch servi, sur un tour du
/// domaine Fallthrough. Branche 3, `reason = "turn_forbids_dispatch"`.
#[test]
fn mika2757_cas1_un_outil_de_dispatch_est_desarme_sur_un_tour_fallthrough() {
    let tools = make_registry_with_tool("run_claude_pilot_groom");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let required: HashSet<String> = ["run_claude_pilot_groom".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_claude_pilot_groom"]),
        TICKET_2750_CLOSED_MSG,
        None,
    );

    assert!(
        effective.is_empty(),
        "l'outil est servi, mais ce tour interdit qu'il réussisse — \
             exigence restée : {effective:?}"
    );
}

/// Cas 2 — le contrôle négatif humain. Même outil, même registre, même
/// tableau servi : seul le message change, et il suffit à ne rien désarmer.
#[test]
fn mika2757_cas2_la_meme_exigence_survit_a_une_demande_humaine() {
    let tools = make_registry_with_tool("run_claude_pilot_groom");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let required: HashSet<String> = ["run_claude_pilot_groom".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_claude_pilot_groom"]),
        "groom mika#2750",
        None,
    );

    assert_eq!(
        effective,
        HashSet::from(["run_claude_pilot_groom".to_string()]),
        "hors domaine webhook, le tour n'interdit rien"
    );
}

/// Cas 3 — la branche 2, celle que la branche 1 ne peut pas voir : l'outil
/// **existe** dans le registre et n'est pourtant pas dans le tableau servi.
/// C'est la forme que produisent les quatre couches de retrait (denylist
/// d'identité, `FALLTHROUGH_WITHHELD_TOOLS`, `filter_ci_excluded_tools`,
/// filtre provider compact). `reason = "not_served"`, et ce n'est pas un
/// défaut de configuration.
#[test]
fn mika2757_cas3_un_outil_existant_mais_non_servi_est_desarme() {
    let tools = make_registry_with_tool("create_task");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let required: HashSet<String> = ["create_task".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        // Le tour ne sert pas `create_task`, alors que le registre le connaît.
        &served(&["run_gh"]),
        TICKET_2750_CLOSED_MSG,
        None,
    );

    assert!(
        effective.is_empty(),
        "un outil absent du tableau servi ne peut pas être exigé — \
             exigence restée : {effective:?}"
    );
}

/// Cas 4 — un tour webhook qui n'exige **pas** d'outil de dispatch. Le
/// domaine n'entre même pas en jeu (`[GitHub] PR ` est territoire qa), mais
/// surtout : `qa_pr_view` et `run_gh` ne sont pas de la paire, donc la
/// branche 3 ne les regarde pas. Un désarmement qui les toucherait casserait
/// la revue de PR.
#[test]
fn mika2757_cas4_un_tour_de_revue_garde_ses_exigences_qa() {
    let tools = ToolRegistry::new();
    let qa_pr_view = make_resolved_skill_tool("qa_pr_view");
    let run_gh = make_resolved_skill_tool("run_gh");
    let mut skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    skill_map.insert("qa_pr_view".to_string(), &qa_pr_view);
    skill_map.insert("run_gh".to_string(), &run_gh);

    let required: HashSet<String> = ["qa_pr_view".to_string(), "run_gh".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["qa_pr_view", "run_gh"]),
        "[GitHub] PR review (approved) on senara-solutions/mika#2756 — vol 1 de mika#2755",
        None,
    );

    assert_eq!(
        effective, required,
        "aucun des deux n'est un outil de dispatch"
    );
}

/// Cas 5 — le tour où le dispatch est l'ordre même. Un `ready`-label sort du
/// domaine Fallthrough par construction
/// (`READY_LABEL_DISPATCH_MARKER`), donc `run_claude_pilot` y reste exigé.
/// Si ce cas rougissait, le désarmement aurait désarmé la boucle autonome
/// elle-même.
#[test]
fn mika2757_cas5_un_tour_ready_label_garde_son_exigence_de_dispatch() {
    let tools = make_registry_with_tool("run_claude_pilot");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let required: HashSet<String> = ["run_claude_pilot".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        None,
        &served(&["run_claude_pilot"]),
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — un ticket prêt",
        None,
    );

    assert_eq!(
        effective,
        HashSet::from(["run_claude_pilot".to_string()]),
        "sur un ready-label, le dispatch est l'ordre du tour, pas son interdit"
    );
}

// -- raison_de_desarmement tests (mika#2814 vol 4) --

/// Manager MCP sans connexion portant un seul outil namespacé au grade
/// donné — le harnais des sept cas ci-dessous.
fn mcp_avec_outil(outil: &str, grade: crate::skills::manifest::DataGrade) -> McpManager {
    let (serveur, nom) = outil
        .strip_prefix("mcp__")
        .and_then(|r| r.split_once("__"))
        .expect("un nom d'outil MCP namespacé");
    McpManager::sans_connexion_pour_tests(
        HashMap::from([(outil.to_string(), (serveur.to_string(), nom.to_string()))]),
        HashMap::from([(outil.to_string(), grade)]),
    )
}

/// Le `false` d'avant mika#3004, rendu par les prédicats eux-mêmes.
fn tour_neutre() -> ProprietesDuTour {
    ProprietesDuTour::depuis(None, "classe les notes de la semaine")
}

/// **Cas 1 — le cœur de l'AC, et l'épinglage de l'ordre des branches.**
///
/// Un outil MCP testimony n'est **jamais** servi : `retain_publiables`
/// (vol 1) l'a retiré du lot publié, donc de `enabled_tool_names`. Cet état
/// — *testimony **et** non servi* — est donc le seul que la production
/// produise, et c'est exactement celui sur lequel une branche
/// `BanTestimony` posée **après** `NonServi` serait inatteignable à 100 %,
/// pour toujours, avec tous les tests verts.
///
/// Ce test est ce qui l'empêche : il rougit sur l'ordre faux.
#[test]
fn mika2814_un_outil_mcp_testimony_non_servi_est_desarme_sous_le_ban() {
    use crate::skills::manifest::DataGrade;

    let outil = "mcp__journal-perso__read_entry";
    let mcp = mcp_avec_outil(outil, DataGrade::Testimony);
    let tools = ToolRegistry::new();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let rien = served(&[]);

    assert_eq!(
        raison_de_desarmement(outil, &tools, &skill_map, Some(&mcp), &rien, tour_neutre()),
        Some(RaisonDesarmement::BanTestimony),
        "la raison doit nommer le ban doctrinal, pas `NonServi` — deux populations \
             sous les mêmes octets est la classe mika#2205, et une branche testimony \
             posée après `NonServi` est inatteignable par construction (plan D3)"
    );
}

/// **Cas 2 — l'état que la production ne produit pas, et qui tient quand
/// même.** Si un jour `retain_publiables` cesse de retirer l'outil, ou si un
/// troisième site publie sans filtrer, l'outil testimony arrive *servi* ici.
/// La réponse doit être la même : le ban ne dépend pas du tableau servi. Sans
/// ce cas, la seule garantie serait un effet de bord du vol 1.
#[test]
fn mika2814_un_outil_mcp_testimony_servi_reste_desarme_sous_le_ban() {
    use crate::skills::manifest::DataGrade;

    let outil = "mcp__journal-perso__read_entry";
    let mcp = mcp_avec_outil(outil, DataGrade::Testimony);
    let tools = ToolRegistry::new();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    assert_eq!(
        raison_de_desarmement(
            outil,
            &tools,
            &skill_map,
            Some(&mcp),
            &served(&[outil]),
            tour_neutre()
        ),
        Some(RaisonDesarmement::BanTestimony),
        "le ban est une propriété du grade du serveur, pas du tableau servi"
    );
}

/// **Cas 3 — le contrôle négatif du grade.** Même outil MCP, même manager,
/// même tableau servi : seul `data_grade` change, et l'exigence survit
/// intacte. C'est ce cas qui interdit à la branche 2 de bannir *tout* outil
/// MCP, et il est la moitié du couple auto-nettoyant de la Fire-Disposition
/// avec le cas 4.
#[test]
fn mika2814_un_outil_mcp_operational_servi_nest_pas_desarme() {
    use crate::skills::manifest::DataGrade;

    let outil = "mcp__calendrier__list_events";
    let mcp = mcp_avec_outil(outil, DataGrade::Operational);
    let tools = ToolRegistry::new();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    assert_eq!(
        raison_de_desarmement(
            outil,
            &tools,
            &skill_map,
            Some(&mcp),
            &served(&[outil]),
            tour_neutre()
        ),
        None,
        "un outil MCP `operational` servi est une exigence satisfaisable — c'est \
             exactement ce que le refus de la garde proposée par le vol 3 protège (plan D7)"
    );
}

/// **Cas 4 — l'autre moitié du couple.** Outil MCP `operational` mais non
/// servi : la raison doit rester `NonServi`. Le ban ne se substitue pas à la
/// branche 3 pour les outils MCP en général.
#[test]
fn mika2814_un_outil_mcp_operational_non_servi_est_desarme_comme_non_servi() {
    use crate::skills::manifest::DataGrade;

    let outil = "mcp__calendrier__list_events";
    let mcp = mcp_avec_outil(outil, DataGrade::Operational);
    let tools = ToolRegistry::new();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let rien = served(&[]);

    assert_eq!(
        raison_de_desarmement(outil, &tools, &skill_map, Some(&mcp), &rien, tour_neutre()),
        Some(RaisonDesarmement::NonServi),
        "un outil MCP non banni et non servi relève de la branche 3, pas de la 2"
    );
}

/// **Cas 5 — la branche 1 garde sa préséance.** Aucun manager, registre vide,
/// skill map vide : l'outil n'existe nulle part, et c'est `Inexistant` qui
/// répond — pas `NonServi`, bien qu'il ne soit pas servi non plus. C'est ce
/// qui épingle que `BanTestimony` se pose **après** la branche 1 : un outil
/// qui n'existe pas n'a pas de grade.
#[test]
fn mika2814_un_outil_inexistant_reste_desarme_comme_inexistant() {
    let tools = ToolRegistry::new();
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    assert_eq!(
        raison_de_desarmement(
            "mcp__journal-perso__read_entry",
            &tools,
            &skill_map,
            None,
            &served(&[]),
            tour_neutre()
        ),
        Some(RaisonDesarmement::Inexistant),
        "sans manager MCP, un nom namespacé n'existe pas plus qu'un autre — la \
             branche 1 reste première, et son `WARN` accuse la config de la skill"
    );
}

/// **Cas 6 — la branche 4 est inchangée.** Un outil de dispatch servi, sur un
/// tour dont le domaine interdit le dispatch : `TourInterditDispatch`. Les
/// propriétés du tour entrent déjà calculées (mika#2757 — le domaine est une
/// propriété du tour), et l'insertion de la branche 2 ne les a pas déplacées.
/// Depuis mika#3004 le texte de l'`issues.closed` remplace le `true` littéral :
/// c'est le prédicat qui décide, et la branche 5 ne peut pas voler la réponse.
#[test]
fn mika2814_un_outil_de_dispatch_reste_desarme_par_le_tour() {
    let tools = make_registry_with_tool("run_claude_pilot_groom");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    assert_eq!(
        raison_de_desarmement(
            "run_claude_pilot_groom",
            &tools,
            &skill_map,
            None,
            &served(&["run_claude_pilot_groom"]),
            ProprietesDuTour::depuis(None, TICKET_2750_CLOSED_MSG)
        ),
        Some(RaisonDesarmement::TourInterditDispatch),
        "la branche du tour reste la dernière, et elle répond toujours"
    );
}

/// **Cas 7 — le contrôle négatif global.** Un builtin servi sur un tour
/// neutre : rien ne désarme. Sans ce cas, une fonction qui rendrait
/// `Some(_)` pour tout serait verte sur les six autres.
#[test]
fn mika2814_un_builtin_servi_sur_un_tour_neutre_nest_pas_desarme() {
    let tools = make_registry_with_tool("create_task");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();

    assert_eq!(
        raison_de_desarmement(
            "create_task",
            &tools,
            &skill_map,
            None,
            &served(&["create_task"]),
            tour_neutre()
        ),
        None,
        "un builtin servi hors domaine interdit est une exigence tenable"
    );
}

/// **Le cas bout en bout (livrable 6).** Les sept cas ci-dessus interrogent
/// la fonction qui *décide* ; celui-ci interroge la fonction qui *filtre*,
/// pour que le câblage du `match` soit lui aussi tenu — un bras qui rendrait
/// `true` au lieu de `false` passerait les sept et échouerait ici.
///
/// Les deux exigences voyagent **ensemble**, parce qu'un lot vide et un lot
/// correctement amputé ne doivent pas rendre les mêmes octets : le builtin
/// survivant est ce qui distingue « le ban a retiré l'outil testimony » de
/// « le filtre a tout retiré ».
#[test]
fn mika2814_le_filtre_ampute_lexigence_testimony_et_garde_le_builtin() {
    use crate::skills::manifest::DataGrade;

    let outil_testimony = "mcp__journal-perso__read_entry";
    let mcp = mcp_avec_outil(outil_testimony, DataGrade::Testimony);
    let tools = make_registry_with_tool("create_task");
    let skill_map: HashMap<String, &ResolvedSkillTool> = HashMap::new();
    let required: HashSet<String> = [outil_testimony.to_string(), "create_task".to_string()].into();

    let effective = filter_available_required_tools(
        &required,
        &tools,
        &skill_map,
        Some(&mcp),
        &served(&["create_task"]),
        "classe les notes de la semaine",
        None,
    );

    assert_eq!(
        effective,
        HashSet::from(["create_task".to_string()]),
        "l'exigence testimony doit disparaître du lot effectif et le builtin y rester \
             — un lot vide dirait autre chose (classe mika#2205)"
    );
}
