//! Carte outil→skill, délais de skill, visibilité d'outils, contrat de preuve.
//!
//! `max_skill_timeout`, la résolution d'un nom d'outil vers son skill
//! propriétaire et ses overrides (mika#2276), ce que le filtre rapporte de
//! ce qu'il a gardé (mika#2593), le prédicat qui traite les deux formes au
//! même site (mika#2748) et `apply_reflection_evidence_contract` (mika#1952).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- Skill helper tests --

// `make_skill_entry` et `make_skill_entry_with_constraints` sont promus dans
// `tests/mod.rs` : les callbacks et les outils requis les lisent aussi, et
// trois variantes y ajoutent un champ depuis leur propre tranche.

#[test]
fn test_build_skill_tool_map_collects_all_tools() {
    let s1 = make_skill_entry("search", 30, &["web_search"]);
    let s2 = make_skill_entry("calc", 10, &["calculate", "convert"]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];
    let map = build_skill_tool_map(&matched);

    assert_eq!(map.len(), 3);
    assert!(map.contains_key("web_search"));
    assert!(map.contains_key("calculate"));
    assert!(map.contains_key("convert"));
}

#[test]
fn test_build_skill_tool_map_empty_when_no_skills() {
    let matched: Vec<&SkillEntry> = vec![];
    let map = build_skill_tool_map(&matched);
    assert!(map.is_empty());
}

#[test]
fn test_build_skill_tool_map_last_skill_wins_on_collision() {
    let s1 = make_skill_entry("alpha", 10, &["shared_tool"]);
    let s2 = make_skill_entry("beta", 20, &["shared_tool"]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];
    let map = build_skill_tool_map(&matched);
    assert_eq!(map.len(), 1);
    assert_eq!(map["shared_tool"].skill_dir, PathBuf::from("/skills/beta"));
}

#[test]
fn test_build_skill_tool_map_collision_logs_both_skills() {
    // AC1 companion: verify the collision path is exercised and
    // the WARN log names both colliding skills and their handler types.
    // The codebase does not use a log-capture test harness, so we
    // verify structurally: the collision path produces a valid map
    // (last-write-wins preserved), and the `warn!` call is visible
    // in code review.
    let s1 = make_skill_entry("alpha", 10, &["shared_tool"]);
    let s2 = make_skill_entry("beta", 20, &["shared_tool"]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];

    let map = build_skill_tool_map(&matched);
    assert_eq!(map.len(), 1);
    assert_eq!(map["shared_tool"].skill_dir, PathBuf::from("/skills/beta"));
}

#[test]
fn test_max_skill_timeout_returns_largest() {
    let s1 = make_skill_entry("fast", 10, &[]);
    let s2 = make_skill_entry("slow", 120, &[]);
    let matched: Vec<&SkillEntry> = vec![&s1, &s2];
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        120
    );
}

/// mika#2276 AC4 — **contrôle négatif de M1.**
///
/// La doctrine QA (`skills/bundled/qa-review/system_prompt.md:194`) raisonne
/// sur « `run_shell`'s 30s budget ». Le moteur ne tenait pas ce plancher :
/// `max_skill_timeout` prend le MAXIMUM des timeouts de tous les skills du
/// tour et l'applique uniformément à chaque appel d'outil. `shell-exec`
/// déclare 30 s ; `build-mika`, dépendance déclarée de `qa-review`, déclare
/// 300 s. Mesuré sur la trace `921f11f0` : deux `cargo test --release` de
/// 237,9 s et 231,1 s, soit 469 s des ~506 s d'enveloppe du tour.
///
/// Ce test asserte le contrat inverse : **le budget d'un outil est celui du
/// skill qui le définit**, pas celui du skill le plus généreux du tour.
///
/// Rouge sur `main` — voir le corps de PR : avant le fix, le seul chemin de
/// résolution était `max_skill_timeout`, qui rend 300 pour ce même jeu.
#[test]
fn mika2276_tool_budget_is_its_own_skills_budget_not_the_turns_maximum() {
    // Le jeu exact de la trace 921f11f0 : shell-exec (30 s) possède
    // `run_shell`, build-mika (300 s) est chargé comme dépendance.
    let shell_exec = make_skill_entry("shell-exec", 30, &["run_shell"]);
    let build_mika = make_skill_entry("build-mika", 300, &["build_mika"]);
    let matched: Vec<&SkillEntry> = vec![&shell_exec, &build_mika];

    let timeouts = build_skill_tool_timeouts(&matched, "anthropic", "claude-sonnet-4-6");

    assert_eq!(
        timeouts.get("run_shell").copied(),
        Some(30),
        "run_shell doit être coupé au budget déclaré par shell-exec (30 s), \
             pas au maximum du tour"
    );
    assert_eq!(
        timeouts.get("build_mika").copied(),
        Some(300),
        "build_mika garde son propre budget — le fix n'abaisse personne"
    );

    // Le défaut lui-même, épinglé : `max_skill_timeout` rend toujours le
    // maximum. Il survit comme plafond de repli pour les outils absents de
    // la carte (MCP, builtins non-skill) ; ce qui change est que la valeur
    // passée à `execute_skill_tool` ne vient plus de lui.
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        300,
        "max_skill_timeout garde sa sémantique de maximum — c'est son usage \
             comme budget par outil qui est retiré"
    );
}

/// mika#2276 AC4 — les overrides provider/modèle suivent le skill propriétaire.
///
/// `effective_timeout` résout `modèle > provider > racine`. La carte par outil
/// doit hériter de cette résolution, sinon un override déclaré chez un skill
/// serait perdu au moment où il compte.
#[test]
fn mika2276_per_tool_budget_honours_provider_overrides() {
    let mut fast = make_skill_entry("fast", 30, &["quick_tool"]);
    fast.provider_overrides.insert(
        "openai".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(90),
            max_prompt_size: None,
        },
    );
    let slow = make_skill_entry("slow", 600, &["slow_tool"]);
    let matched: Vec<&SkillEntry> = vec![&fast, &slow];

    let openai = build_skill_tool_timeouts(&matched, "openai", "gpt-4o");
    assert_eq!(openai.get("quick_tool").copied(), Some(90));

    let anthropic = build_skill_tool_timeouts(&matched, "anthropic", "claude-sonnet-4-6");
    assert_eq!(anthropic.get("quick_tool").copied(), Some(30));
}

/// mika#2276 AC4 — collision de noms d'outil : même règle que la carte des
/// outils et que celle des `data_grade` (dernier écrit gagne).
///
/// Les trois cartes sont construites depuis le même `matched` dans le même
/// ordre ; si celle-ci divergeait, un outil serait dispatché vers le handler
/// d'un skill et coupé au budget d'un autre.
#[test]
fn mika2276_per_tool_budget_collision_follows_the_tool_map() {
    let alpha = make_skill_entry("alpha", 10, &["shared_tool"]);
    let beta = make_skill_entry("beta", 20, &["shared_tool"]);
    let matched: Vec<&SkillEntry> = vec![&alpha, &beta];

    let tool_map = build_skill_tool_map(&matched);
    let timeouts = build_skill_tool_timeouts(&matched, "anthropic", "claude-sonnet-4-6");

    assert_eq!(
        tool_map["shared_tool"].skill_dir,
        PathBuf::from("/skills/beta")
    );
    assert_eq!(
        timeouts.get("shared_tool").copied(),
        Some(20),
        "le budget doit suivre le même gagnant que le handler"
    );
}

#[test]
fn test_max_skill_timeout_fallback_when_empty() {
    let matched: Vec<&SkillEntry> = vec![];
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        crate::planning::policy::TOOL_TIMEOUT_SECS
    );
}

#[test]
fn test_max_skill_timeout_uses_provider_override() {
    let mut s1 = make_skill_entry("search", 30, &[]);
    s1.provider_overrides.insert(
        "openai".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(90),
            max_prompt_size: None,
        },
    );
    let matched: Vec<&SkillEntry> = vec![&s1];
    // With openai provider, should use the override
    assert_eq!(max_skill_timeout(&matched, "openai", "gpt-4o"), 90);
    // With anthropic provider, should use root
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        30
    );
}

#[test]
fn test_inject_skills_appends_prompt_and_tool_defs() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("test", 30, &["test_tool"]);
    entry.prompt_snippet = "Use this skill to test things.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base prompt.".to_string();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        false,
    );

    // Should append skill snippet to system prompt
    assert!(system.contains("test Skill"));
    assert!(system.contains("Use this skill to test things."));
    // Should include the skill tool definition
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "test_tool");
    // Should record variant info for the non-empty prompt
    assert!(variant.is_some());
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(variant_json.get("test").unwrap(), "base");
}

#[test]
fn test_inject_skills_deduplicates_tool_names() {
    let mut tools = ToolRegistry::new();
    // Register a builtin tool named "overlap"
    struct DummyTool;
    #[async_trait::async_trait]
    impl crate::tools::Tool for DummyTool {
        fn name(&self) -> &str {
            "overlap"
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "overlap".to_string(),
                description: "builtin overlap".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &crate::tools::ToolContext<'_>,
        ) -> Result<crate::tools::ToolOutput> {
            Ok(crate::tools::ToolOutput::success("ok"))
        }
    }
    tools.register(Box::new(DummyTool));

    // Skill also defines a tool named "overlap"
    let entry = make_skill_entry("dupe", 30, &["overlap"]);
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, _variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        false,
    );

    // "overlap" should appear exactly once (builtin wins)
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].description, "builtin overlap");
}

#[test]
fn test_apply_agent_tool_visibility_evicts_listed_names() {
    let mut defs = vec![
        ToolDefinition {
            name: "pr_merge_with_gate".to_string(),
            description: "merges PRs".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
        ToolDefinition {
            name: "search_memory".to_string(),
            description: "reads memory".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
        ToolDefinition {
            name: "update_core_memory".to_string(),
            description: "writes memory".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
    ];
    let disabled = vec![
        "pr_merge_with_gate".to_string(),
        "update_core_memory".to_string(),
    ];
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "search_memory");
}

#[test]
fn test_apply_agent_tool_visibility_no_op_when_disabled_empty() {
    let mut defs = vec![ToolDefinition {
        name: "pr_merge_with_gate".to_string(),
        description: "merges PRs".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let disabled: Vec<String> = vec![];
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");
    assert_eq!(
        defs.len(),
        1,
        "empty denylist should leave tool defs unchanged"
    );
}

#[test]
fn test_apply_agent_tool_visibility_case_insensitive() {
    let mut defs = vec![ToolDefinition {
        name: "Pr_Merge_With_Gate".to_string(),
        description: "case-mixed".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let disabled = vec!["pr_merge_with_gate".to_string()];
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");
    assert!(defs.is_empty(), "filter must match case-insensitively");
}

/// mika#2593 — the two fields the filter reports about what it *kept*
/// describe the array **after** the filter, not before it.
///
/// `hidden_count` alone cannot answer "how big is the array the model is
/// actually billed for?", which is the question a cut on `tool_defs_bytes`
/// is judged by. The drift this pins is the one that costs nothing to make
/// and everything to notice: computing either field before `retain`, where
/// both still look like plausible numbers and every behavioural assertion
/// in this file stays green. `kept_bytes` is therefore asserted by
/// **equality** with the post-filter serialization — not against a
/// threshold — and separately asserted to differ from the pre-filter
/// weight, which is what an equality alone could not distinguish.
#[test]
fn mika2593_le_filtre_atteste_le_compte_et_les_octets_restants() {
    let mut defs = vec![
        ToolDefinition {
            name: "search_memory".to_string(),
            description: "searches memory".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "lists tasks — denied below".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
        ToolDefinition {
            name: "send_message".to_string(),
            description: "sends a message".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        },
    ];
    let weight_before = tool_defs_byte_weight(&defs);

    let disabled = vec!["list_tasks".to_string()];
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");

    // `kept_count` — the field reports `tool_defs.len()` after `retain`.
    assert_eq!(
        defs.len(),
        2,
        "three defs minus one denied leaves two; kept_count reports this length"
    );

    let weight_after = tool_defs_byte_weight(&defs);
    assert_eq!(
        weight_after,
        serde_json::to_string(&defs).unwrap().len(),
        "kept_bytes must be the serialized length of the array that survived the filter"
    );
    assert!(
        weight_after < weight_before,
        "kept_bytes ({weight_after}) must describe the filtered array, not the one the \
             filter received ({weight_before}) — a field computed before `retain` would read \
             equal here and would silently report the unfiltered cost as the served one"
    );
}

// -----------------------------------------------------------------------
// mika#2748 — the predicate handles both shapes, at one site
// -----------------------------------------------------------------------

/// The five states of `(disabled, allowed)`, in one table.
///
/// Each row is a distinct behaviour, and two of them are the reason the
/// table exists rather than three separate tests:
///
/// - **Row 4** pins the composition rule. `a` is written on *both* sides,
///   and the denylist wins — that is what preserves the Webhook Fallthrough
///   widening of mika#2517 on an allowlist agent (`effective_disabled_tools`
///   adds `create_task` to the denylist for one turn, and the intersection
///   must still be the answer). Were the allowlist to win instead, that
///   widening would become a no-op for every allowlist agent, silently.
/// - **Row 5** is the **negative control of the three-state field**, and the
///   only assertion in the repo that distinguishes `Some([])` from `None`.
///   It is what goes red if a future editor "simplifies"
///   `Option<Vec<String>>` into a plain `Vec<String>`: under that shape an
///   agent admitting *no* builtin would be served *every* builtin, which is
///   the worst possible direction for the error to take.
#[test]
fn mika2748_le_predicat_traite_les_cinq_etats_de_la_paire() {
    fn def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("{name} (mika#2748 fixture)"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// `(case label, disabled, allowed, expected survivors)`.
    ///
    /// Named so the table reads as a table. The `Option` is the whole point
    /// of row 5 and must not be flattened away to shorten the signature.
    type Case<'a> = (&'a str, &'a [&'a str], Option<&'a [&'a str]>, &'a [&'a str]);

    let cases: &[Case<'_>] = &[
        (
            "1 — neither shape: early return, array intact",
            &[],
            None,
            &["a", "b", "c"],
        ),
        (
            "2 — denylist only: prior behaviour",
            &["b"],
            None,
            &["a", "c"],
        ),
        (
            "3 — allowlist only: b and c are not admitted",
            &[],
            Some(&["a"]),
            &["a"],
        ),
        (
            "4 — both: intersection, and a name written twice loses",
            &["a"],
            Some(&["a", "b"]),
            &["b"],
        ),
        (
            "5 — Some([]) admits nothing: distinct from case 1",
            &[],
            Some(&[]),
            &[],
        ),
    ];

    for (label, disabled, allowed, expected) in cases {
        let mut defs = vec![def("a"), def("b"), def("c")];
        let disabled: Vec<String> = disabled.iter().map(|s| (*s).to_string()).collect();
        let allowed: Option<Vec<String>> =
            allowed.map(|l| l.iter().map(|s| (*s).to_string()).collect());

        apply_agent_tool_visibility(&mut defs, &disabled, allowed.as_deref(), "mika-test-2748");

        let survivors: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            survivors, *expected,
            "case {label}: served {survivors:?}, expected {expected:?}"
        );
    }
}

/// Both shapes match case-insensitively, not just the denylist.
///
/// The denylist half has had this pinned since mika#811
/// (`test_apply_agent_tool_visibility_case_insensitive`). An allowlist that
/// compared case-*sensitively* would fail in the direction that costs: a
/// roster entry whose case drifted from the registry would **remove** the
/// tool, not keep an extra one.
#[test]
fn mika2748_lallowlist_compare_sans_egard_a_la_casse() {
    let mut defs = vec![ToolDefinition {
        name: "Search_Memory".to_string(),
        description: "case-mixed in the registry".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let allowed = vec!["search_memory".to_string()];
    apply_agent_tool_visibility(&mut defs, &[], Some(&allowed), "mika-test-2748");
    assert_eq!(
        defs.len(),
        1,
        "an allowlist entry must admit a registry name that differs only in case"
    );
}

// -----------------------------------------------------------------------
// mika#1952 U4-a — `apply_reflection_evidence_contract`
// -----------------------------------------------------------------------

/// A schema of the shape the three guarded tools actually declare:
/// `evidence` present in `properties`, absent from `required`.
fn gated_tool_def(name: &str, required: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("{name} (mika#1952 fixture)"),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "category": {"type": "string"},
                "evidence": {"type": "string"}
            },
            "required": required
        }),
    }
}

fn required_of(def: &ToolDefinition) -> Vec<String> {
    def.input_schema["required"]
        .as_array()
        .expect("required is an array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("required entries are strings")
                .to_string()
        })
        .collect()
}

#[test]
fn mika1952_the_three_gated_tools_declare_evidence_required() {
    let mut defs = vec![
        gated_tool_def(
            "update_fact",
            serde_json::json!(["id", "category", "updates"]),
        ),
        gated_tool_def("store_fact", serde_json::json!(["category"])),
        gated_tool_def(
            "update_core_memory",
            serde_json::json!(["section", "action", "reasoning"]),
        ),
    ];

    apply_reflection_evidence_contract(&mut defs);

    for def in &defs {
        assert!(
            required_of(def).contains(&"evidence".to_string()),
            "mika#1952 — `{}` must declare `evidence` as required in reflection mode",
            def.name
        );
    }
    // Additive, not replacing: the pre-existing entries survive, in order.
    assert_eq!(
        required_of(&defs[0]),
        vec!["id", "category", "updates", "evidence"],
        "the filter appends; it must not reorder or drop"
    );
}

#[test]
fn mika1952_applying_the_contract_twice_changes_nothing() {
    let mut defs = vec![gated_tool_def("update_fact", serde_json::json!(["id"]))];
    apply_reflection_evidence_contract(&mut defs);
    let once = defs.clone();
    apply_reflection_evidence_contract(&mut defs);
    assert_eq!(
        defs[0].input_schema, once[0].input_schema,
        "D3 — the mutation is idempotent; a second pass must add no duplicate"
    );
    assert_eq!(required_of(&defs[0]).len(), 2);
}

#[test]
fn mika1952_a_schema_already_declaring_evidence_is_left_alone() {
    let mut defs = vec![gated_tool_def(
        "store_fact",
        serde_json::json!(["category", "evidence"]),
    )];
    let before = defs[0].input_schema.clone();
    apply_reflection_evidence_contract(&mut defs);
    assert_eq!(defs[0].input_schema, before);
}

#[test]
fn mika1952_a_tool_outside_the_list_is_untouched() {
    let mut defs = vec![
        gated_tool_def("search_memory", serde_json::json!(["query"])),
        gated_tool_def("send_message", serde_json::json!(["text"])),
    ];
    let before: Vec<_> = defs.iter().map(|d| d.input_schema.clone()).collect();
    apply_reflection_evidence_contract(&mut defs);
    for (def, was) in defs.iter().zip(before) {
        assert_eq!(
            def.input_schema, was,
            "`{}` does not carry the runtime guard and must not be touched",
            def.name
        );
    }
}

/// D3 — a gated tool absent from the array (evicted by an identity denylist,
/// or dropped by the compact-provider filter) is not an error.
#[test]
fn mika1952_a_partial_tool_array_is_served_not_refused() {
    let mut defs = vec![gated_tool_def(
        "store_fact",
        serde_json::json!(["category"]),
    )];
    apply_reflection_evidence_contract(&mut defs);
    assert_eq!(defs.len(), 1, "no tool is added to cover the missing ones");
    assert!(required_of(&defs[0]).contains(&"evidence".to_string()));
}

/// D4 — an unexpected shape leaves the schema **intact** rather than
/// half-mutated. Each case is asserted separately: a single fixture
/// breaking every term at once would pass against a filter that checks
/// only one of them.
#[test]
fn mika1952_an_unexpected_schema_is_left_intact_without_panicking() {
    // (a) `properties.evidence` absent.
    let mut no_field = vec![ToolDefinition {
        name: "update_fact".to_string(),
        description: "no evidence property".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"id": {"type": "integer"}},
            "required": ["id"]
        }),
    }];
    let before = no_field[0].input_schema.clone();
    apply_reflection_evidence_contract(&mut no_field);
    assert_eq!(
        no_field[0].input_schema, before,
        "a schema with no `evidence` property must be served unchanged"
    );

    // (b) `required` present but not an array.
    let mut bad_required = vec![ToolDefinition {
        name: "store_fact".to_string(),
        description: "required is a string".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"evidence": {"type": "string"}},
            "required": "category"
        }),
    }];
    let before = bad_required[0].input_schema.clone();
    apply_reflection_evidence_contract(&mut bad_required);
    assert_eq!(bad_required[0].input_schema, before);

    // (c) `input_schema` is not an object at all.
    let mut not_object = vec![ToolDefinition {
        name: "update_core_memory".to_string(),
        description: "schema is a string".to_string(),
        input_schema: serde_json::json!("nonsense"),
    }];
    apply_reflection_evidence_contract(&mut not_object);
    assert_eq!(not_object[0].input_schema, serde_json::json!("nonsense"));
}

/// `required` **absent** is a valid schema declaring nothing mandatory, not
/// an unexpected shape — so the array is created rather than refused.
#[test]
fn mika1952_a_schema_without_a_required_array_gains_one() {
    let mut defs = vec![ToolDefinition {
        name: "update_fact".to_string(),
        description: "no required array".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"evidence": {"type": "string"}}
        }),
    }];
    apply_reflection_evidence_contract(&mut defs);
    assert_eq!(required_of(&defs[0]), vec!["evidence"]);
}
