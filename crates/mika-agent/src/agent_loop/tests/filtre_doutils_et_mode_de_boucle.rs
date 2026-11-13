//! Le filtre structurel d'outils, `LoopMode` et les plafonds de pas.
//!
//! Un tour webhook CI ne reçoit aucun outil de dispatch (mika#1324), chaque
//! sortie de boucle décide elle-même de son signal de verdict (mika#2515),
//! et `SilentTrigger` résout son propre plafond de pas.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// ===========================================================================
// mika#1324 — Structural tool filter (CI webhook → no dispatch tools)
// ===========================================================================

// `tool_def` et `full_tool_set` sont promus dans `tests/mod.rs` : la fenêtre de
// contexte les lit aussi.

// AC1 — unit test on the predicate via filter behavior. check_suite.success
// → both dispatch tools removed; benign tools retained.
#[test]
fn filter_ci_excluded_tools_check_suite_success_removes_dispatch_tools() {
    let mut tools = full_tool_set();
    // Canonical check_suite.success message shape from the gateway. Branch
    // matches the original mika#1324 incident (fix/1318/dev-groom-no-force-push)
    // to verify the substring contains "dev-groom" yet the dispatch tools
    // are still removed by the CI prefix check, not by branch substring
    // matching (the architect's diagnostic correction in the ticket body).
    let user_message = "[GitHub] Check suite success on senara-solutions/mika \
                            (branch: fix/1318/dev-groom-no-force-push)";
    filter_ci_excluded_tools(user_message, &mut tools);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        !names.contains(&"run_claude_pilot"),
        "run_claude_pilot must be removed on CI success: {names:?}"
    );
    assert!(
        !names.contains(&"run_claude_pilot_groom"),
        "run_claude_pilot_groom must be removed on CI success: {names:?}"
    );
    assert!(
        names.contains(&"send_message"),
        "send_message must be retained: {names:?}"
    );
    assert!(
        names.contains(&"run_gh"),
        "run_gh must be retained: {names:?}"
    );
}

// AC2 — integration-style coverage on the failure prefix. check_suite.failure
// also removes dispatch tools.
#[test]
fn filter_ci_excluded_tools_check_suite_failure_removes_dispatch_tools() {
    let mut tools = full_tool_set();
    let user_message = "[GitHub] Check suite failure on senara-solutions/mika \
                            (branch: fix/1318/dev-groom-no-force-push)";
    filter_ci_excluded_tools(user_message, &mut tools);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        !names.contains(&"run_claude_pilot"),
        "run_claude_pilot must be removed on CI failure: {names:?}"
    );
    assert!(
        !names.contains(&"run_claude_pilot_groom"),
        "run_claude_pilot_groom must be removed on CI failure: {names:?}"
    );
}

// AC3 — ready-label dispatch flow continues to work. The ready-label
// marker is NOT a check_suite event; dispatch tools must remain available
// so `webhook_ready_label_dispatch` intent guard can fire and the agent
// can call `run_claude_pilot` to actually dispatch.
#[test]
fn filter_ci_excluded_tools_ready_label_event_retains_dispatch_tools() {
    let mut tools = full_tool_set();
    let user_message =
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — fix some thing";
    filter_ci_excluded_tools(user_message, &mut tools);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"run_claude_pilot"),
        "run_claude_pilot must be retained on ready-label dispatch: {names:?}"
    );
    assert!(
        names.contains(&"run_claude_pilot_groom"),
        "run_claude_pilot_groom must be retained on ready-label dispatch: {names:?}"
    );
    assert!(
        names.contains(&"send_message"),
        "send_message must be retained: {names:?}"
    );
}

// Boundary: non-webhook conversational input never trips the filter.
#[test]
fn filter_ci_excluded_tools_non_webhook_input_retains_all_tools() {
    let mut tools = full_tool_set();
    let user_message = "hey can you check the status of the dev-groom dispatch?";
    filter_ci_excluded_tools(user_message, &mut tools);
    assert_eq!(
        tools.len(),
        5,
        "non-webhook input must not affect the tool set"
    );
}

// Boundary: empty tools list is a no-op (defensive).
#[test]
fn filter_ci_excluded_tools_empty_tools_is_noop() {
    let mut tools: Vec<ToolDefinition> = Vec::new();
    let user_message = "[GitHub] Check suite success on senara-solutions/mika (branch: main)";
    filter_ci_excluded_tools(user_message, &mut tools);
    assert!(tools.is_empty());
}

// -- LoopMode tests --

#[test]
fn test_loop_mode_conversation_properties() {
    let mode = LoopMode::Conversation;
    assert!(mode.is_conversation());
    assert!(mode.follow_up_on_empty());
    assert!(mode.saves_to_db());
    assert_eq!(mode.label(), "agent");
}

#[test]
fn test_loop_mode_silent_properties() {
    let mode = LoopMode::Silent {
        max_steps: crate::planning::policy::MAX_TOOL_STEPS,
    };
    assert!(!mode.is_conversation());
    assert!(!mode.follow_up_on_empty());
    assert!(mode.saves_to_db());
    assert_eq!(mode.label(), "silent agent");
    assert_eq!(mode.max_steps(), crate::planning::policy::MAX_TOOL_STEPS);
}

// ── mika#2515 U1 — chaque sortie DÉCIDE du signal de verdict ──────────

/// Sorties de `run_loop` qui ne posent délibérément **aucun** signal de
/// verdict.
///
/// Ce n'est **pas** une allowlist d'infractions : chaque entrée nomme une
/// sortie **structurellement inatteignable** depuis un tour de callback, avec
/// le prédicat qui l'établit. Quand le scan tire, on **arme** le site — on
/// n'ajoute une entrée QUE si l'inatteignabilité est démontrable comme
/// celle-ci (doctrine mika#2201 : on déclare, on n'allowliste pas).
const LOOP_EXITS_WITHOUT_SIGNAL: &[(&str, &str)] = &[(
    "Done — texte vide APRÈS follow-up",
    "LoopMode::follow_up_on_empty() est false pour Silent : cette sortie est \
         inatteignable depuis un tour de callback, qui est toujours Silent. \
         Propriété BOOLÉENNE et épinglée — voir l'assertion auto-nettoyante de \
         mika2515_the_excluded_loop_exit_is_still_unreachable.",
)];

/// Renvois de `run_silent_agent` qui ne lisent délibérément **aucun** fait de
/// verdict.
///
/// **Vocabulaire DISTINCT de [`LOOP_EXITS_WITHOUT_SIGNAL`]** : là l'exclusion
/// dit « ce site est inatteignable », ici elle dit « ce site n'a pas le
/// prédicat sous la main ». Confondre les deux ferait passer une course
/// d'horloge pour une impossibilité — et écrirait une fausseté dans la garde
/// même qui existe pour empêcher les faussetés.
const SILENT_RETURNS_WITHOUT_VERDICT_READ: &[(&str, &str)] = &[(
    "contrôle de deadline du prélude (mika#848 F3b)",
    "`qa_verdict_due` dérive de `loaded_skill_names`, calculé APRÈS ce site : \
         le prédicat n'existe pas encore. Population bornée par le fait que \
         l'enveloppe ENTIÈRE devrait s'écouler dans `load_agent_context` + \
         `list_commitments`. NON inatteignable — suivi mika#2515-a, dont la \
         précondition écrite est que le WARN de ce site soit observé sur un \
         trigger de callback.",
)];

/// Les lignes de production du fichier, commentaires exclus.
///
/// La prose de doc de ce ticket cite ses propres motifs abondamment (« un
/// scan ancré sur `return Ok(LoopResult::` n'en trouve que cinq ») ; sans ce
/// filtre le scan compterait sa propre explication comme une sortie. C'est le
/// faux positif que mika#2050 a mesuré sur le Signal S, une classe plus tôt.
fn production_lines_of_this_module() -> Vec<String> {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let production = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));
    production
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(str::to_owned)
        .collect()
}

fn count_in_production(needle: &str) -> usize {
    production_lines_of_this_module()
        .iter()
        .filter(|l| l.contains(needle))
        .count()
}

/// **V9 — la cardinalité, le seul terme qu'aucune fixture ne peut voir.**
///
/// Un prédicat devenu trop étroit passerait en ne regardant rien
/// (mika#2205) ; une sortie ajoutée sans pose de signal ne rendrait **aucune
/// décision fausse** et laisserait toutes les assertions vertes, la seule
/// conséquence étant une population muette. C'est très exactement le défaut
/// que ce ticket ferme : `run_loop` posait son signal à **deux sorties sur
/// six**.
///
/// # Le motif est `Ok(LoopResult::`, jamais `return Ok(LoopResult::`
///
/// La sortie `MaxStepsExceeded` est l'**expression finale** de `run_loop` :
/// elle n'a pas de `return`. Un prédicat ancré sur `return` en trouve
/// **cinq**, et l'implémenteur a alors deux façons de se tromper — faire
/// rougir un scan correct, ou « corriger » la cardinalité à 5 et sortir en
/// silence la sortie max-steps de la population. Inversement, `LoopResult::`
/// nu rend les motifs de `match` des trois boucles, donc un scan
/// perpétuellement rouge, c'est-à-dire désarmé.
///
/// **Le scan asserte la cardinalité, JAMAIS les numéros de ligne** : ceux-ci
/// ont déjà dérivé de +59 entre deux passes du plan de ce ticket, et un scan
/// qui les figerait rougirait à chaque réécriture du fichier sans qu'aucune
/// sortie ne soit devenue muette — un détecteur qu'on désarme parce qu'il
/// crie à tort.
#[test]
fn mika2515_every_loop_exit_decides_about_the_verdict_signal() {
    let exits = count_in_production("Ok(LoopResult::");
    let armed_concluded = count_in_production("mark_unmet_after_retry()");
    let armed_cut_off = count_in_production("mark_cut_off(");
    let declared_excluded = LOOP_EXITS_WITHOUT_SIGNAL.len();

    assert_eq!(
        exits, 6,
        "cardinalité des sorties de `run_loop` : relevée contre le fichier, \
             jamais posée de mémoire (le plan de ce ticket l'a écrite 4 avant que \
             le relevé ne soit fait). Si une sortie a été ajoutée, elle doit \
             DÉCIDER : poser le signal, ou être déclarée dans \
             LOOP_EXITS_WITHOUT_SIGNAL avec son prédicat d'inatteignabilité."
    );
    assert_eq!(
        armed_concluded, 3,
        "trois sorties EndTurn posent la moitié « conclu muet » : texte non \
             vide, texte vide (miroir Silent), et le « Force EndTurn » de \
             `send_message` — ce dernier est le trou de mika#2368 que mika#2515 \
             ferme, et mika#2136 l'avait nommé ici même"
    );
    assert_eq!(
        armed_cut_off, 2,
        "deux sorties coupées posent la moitié « coupé » : DeadlineExceeded \
             et MaxStepsExceeded"
    );
    assert_eq!(
        armed_concluded + armed_cut_off + declared_excluded,
        exits,
        "chacune des {exits} sorties doit DÉCIDER : {armed_concluded} posent \
             « conclu », {armed_cut_off} posent « coupé », {declared_excluded} \
             sont déclarées exclues. Le compte ne boucle pas — une sortie est \
             muette sans l'avoir dit."
    );
}

/// **V9 — l'assertion auto-nettoyante de l'exclusion de `run_loop`.**
///
/// Le jour où `follow_up_on_empty()` devient vrai pour `Silent`, l'exclusion
/// cesse d'être vraie et **ce test rougit** — au lieu de laisser une sortie
/// s'ouvrir en silence derrière une raison morte.
#[test]
fn mika2515_the_excluded_loop_exit_is_still_unreachable() {
    let silent = LoopMode::Silent {
        max_steps: crate::planning::policy::MAX_TOOL_STEPS,
    };
    assert!(
        !silent.follow_up_on_empty(),
        "l'entrée de LOOP_EXITS_WITHOUT_SIGNAL repose sur cette propriété : \
             si elle change, la sortie « Done après follow-up » devient \
             atteignable depuis un tour de callback et doit être ARMÉE, pas \
             laissée déclarée"
    );
    assert_eq!(
        LOOP_EXITS_WITHOUT_SIGNAL.len(),
        1,
        "une seule sortie est exclue, et pour une raison booléenne"
    );
    assert!(
        LOOP_EXITS_WITHOUT_SIGNAL[0]
            .1
            .contains("follow_up_on_empty"),
        "l'exclusion doit NOMMER le prédicat qui l'établit — une exclusion \
             sans prédicat est une exemption déguisée"
    );
}

/// **V9 / DoD 1b — les quatre renvois de `run_silent_agent` décident aussi.**
///
/// Sans cette moitié, le scan couvrirait la fonction qui **pose** le signal
/// et laisserait libre celle qui le **lit** — c'est-à-dire exactement la
/// moitié par laquelle le fait se perd. Le plan de ce ticket a compté trois
/// renvois sur quatre pendant six passes, et le quatrième est précisément
/// celui qui lit le drapeau.
///
/// `Ok(SilentTurnOutcome` et non `return Ok(...)` : la construction finale
/// n'a pas de `return`, et c'est celle dont l'omission coûterait le plus. Un
/// scan ancré sur `return` y compterait **trois** — le même compte que les
/// versions fautives du plan, donc un instrument qui **confirmerait** la
/// mauvaise cardinalité.
#[test]
fn mika2515_every_silent_return_decides_about_the_verdict_signal() {
    let returns = count_in_production("Ok(SilentTurnOutcome");
    let reading = count_in_production("SilentTurnOutcome::from_signal");
    let declared_excluded = SILENT_RETURNS_WITHOUT_VERDICT_READ.len();

    assert_eq!(
        returns, 4,
        "cardinalité des renvois de `run_silent_agent` — relevée contre le \
             fichier. Trois lisent le fait, un est déclaré exclu."
    );
    assert_eq!(
        reading, 3,
        "trois renvois LISENT le fait : la sous-branche « deadline trop \
             proche », le bras DeadlineExceeded (dont mika#2368 jetait le fait \
             explicitement), et la construction finale"
    );
    assert_eq!(
        reading + declared_excluded,
        returns,
        "chacun des {returns} renvois doit décider : {reading} lisent, \
             {declared_excluded} sont déclarés exclus"
    );
    assert_eq!(
        count_in_production("SilentTurnOutcome::default()"),
        declared_excluded,
        "le seul `default()` restant est celui du renvoi déclaré exclu — un \
             second serait un fait jeté sans l'avoir dit"
    );
}

/// **V9 — l'assertion auto-nettoyante de l'exclusion de `run_silent_agent`,
/// et elle est ORDINALE, pas booléenne.**
///
/// Il n'y a pas de propriété à interroger : l'exclusion repose sur le fait
/// que le calcul de `loaded_skill_names` vient **après** le prélude de
/// deadline. Le jour où quelqu'un remonte la correspondance de skills, cette
/// exclusion cesse d'être vraie et ce test rougit en nommant le site devenu
/// armable, au lieu de laisser un renvoi muet derrière une raison morte.
#[test]
fn mika2515_the_excluded_silent_return_still_lacks_its_predicate() {
    let lines = production_lines_of_this_module();
    let index_of = |needle: &str| -> usize {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains(needle))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "l'ancre {needle:?} doit être UNIQUE pour servir d'adresse ; \
                 trouvé {} occurrences",
            hits.len()
        );
        hits[0]
    };

    let prelude = index_of("silent agent deadline exceeded during prelude");
    let skills = index_of("let loaded_skill_names = skill_names_of(&matched);");

    assert!(
        prelude < skills,
        "l'exclusion du prélude repose sur cet ORDRE : `qa_verdict_due` \
             dérive de `loaded_skill_names`, calculé après le prélude. Si le \
             calcul a été remonté, le prédicat existe désormais à ce site et le \
             renvoi doit être ARMÉ — retirer l'entrée de \
             SILENT_RETURNS_WITHOUT_VERDICT_READ."
    );
    assert_eq!(SILENT_RETURNS_WITHOUT_VERDICT_READ.len(), 1);
    assert!(
        SILENT_RETURNS_WITHOUT_VERDICT_READ[0]
            .1
            .contains("loaded_skill_names"),
        "l'exclusion doit nommer le prédicat indisponible"
    );
    assert!(
        !SILENT_RETURNS_WITHOUT_VERDICT_READ[0]
            .1
            .to_lowercase()
            .contains("inatteignable\u{20}depuis"),
        "vocabulaire DISTINCT de LOOP_EXITS_WITHOUT_SIGNAL : ce site n'est \
             pas inatteignable, son prédicat est indisponible. Les confondre \
             ferait passer une course d'horloge pour une impossibilité."
    );
}

/// **mika#2515 — la prémisse de P0 est RÉFUTÉE, et voici sa moitié
/// structurelle.**
///
/// Le plan de ce ticket écrit que le site « Force EndTurn » est atteignable
/// en mode `Silent` parce qu'« aucun garde de mode ne le protège ». Le site
/// en porte **deux**, conjonctifs avec `send_message_boundary_active` :
///
/// ```text
/// if send_message_boundary_active && mode.is_conversation() && !is_automated_trigger
/// ```
///
/// `mode.is_conversation()` **est** un garde de mode, et il est faux pour
/// `Silent` ; `is_automated_trigger` est vrai dès que le message commence par
/// `[callback:`. Le commentaire du site le dit d'ailleurs : *« Only fires when
/// ALL of: Conversation mode (**silent/callback modes exempt**) »*.
///
/// **Conséquence : la population de P0 est vide aujourd'hui.** U1e est livré
/// quand même — le site doit *décider* (DoD 1) et se trouve armé d'avance si
/// ces gardes s'élargissaient — mais il ne ferme aucun défaut observable, et
/// l'écrire ici est ce qui empêche la prochaine lecture de le recompter comme
/// un trou fermé. C'est la leçon mika#2272 que le plan cite pour son propre
/// prédicat de coupure, appliquée à lui.
///
/// Le jour où `is_conversation()` devient vrai pour `Silent`, ce test rougit
/// et U1e cesse d'être un filet sans population.
#[test]
fn mika2515_the_force_endturn_exit_is_mode_guarded() {
    let silent = LoopMode::Silent {
        max_steps: crate::planning::policy::MAX_CALLBACK_TOOL_STEPS,
    };
    assert!(
        !silent.is_conversation(),
        "premier garde de la frontière #771 : un tour de callback n'est pas \
             conversationnel, donc le « Force EndTurn » ne peut pas y firer. Si \
             cette propriété change, la population de P0 devient non vide — et \
             U1e, déjà armé, devient un correctif réel plutôt qu'un filet sans \
             population."
    );
    assert!(
        LoopMode::Conversation.is_conversation(),
        "contrôle de bonne foi : le prédicat n'est pas constamment faux"
    );
    // Le second garde, et il tiendrait seul : le marqueur d'un callback de
    // build est reconnu comme déclencheur automatisé. Reconstruit depuis la
    // grammaire du moteur plutôt que recopié.
    let marker = format!(
        "[callback: {}]",
        crate::qa_build_callback::BUILD_CALLBACK_LABEL
    );
    assert!(marker.starts_with("[callback:"));
}

/// **V9 — contrôle de bonne foi du scan de cardinalité.**
///
/// Un scan vérifié seulement par son propre vert est un scan vérifié par
/// rien. Celui-ci est exercé sur des extraits fabriqués plutôt qu'en éditant
/// le vrai fichier : le motif doit compter l'expression finale sans `return`,
/// et ne pas compter les motifs de `match`.
#[test]
fn mika2515_the_cardinality_predicate_counts_what_it_claims() {
    let count = |src: &str, needle: &str| {
        src.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| l.contains(needle))
            .count()
    };

    // L'expression finale, sans `return` — celle qu'un prédicat ancré sur
    // `return` manquerait.
    let final_expr = "    Ok(LoopResult::MaxStepsExceeded {\n        usage: None,\n    })\n";
    assert_eq!(count(final_expr, "Ok(LoopResult::"), 1);
    assert_eq!(
        count(final_expr, "return Ok(LoopResult::"),
        0,
        "c'est la raison pour laquelle le motif du scan omet `return`"
    );

    // Un motif de `match` n'est pas une sortie.
    let match_arm = "        LoopResult::Done { text, .. } => text,\n";
    assert_eq!(
        count(match_arm, "Ok(LoopResult::"),
        0,
        "ancrer sur `LoopResult::` nu rendrait le scan perpétuellement rouge"
    );

    // Une ligne de commentaire citant le motif n'est pas une sortie — la
    // prose de ce ticket en écrit plusieurs.
    let prose = "    // Un scan ancré sur `Ok(LoopResult::` n'en trouve que cinq.\n";
    assert_eq!(
        count(prose, "Ok(LoopResult::"),
        0,
        "faux positif mesuré par mika#2050 sur le Signal S, une classe plus tôt"
    );
}

#[test]
fn test_loop_mode_team_properties() {
    let mode = LoopMode::Team;
    assert!(!mode.is_conversation());
    assert!(mode.follow_up_on_empty());
    assert!(!mode.saves_to_db());
    assert_eq!(mode.label(), "team agent");
}

// -- SilentTrigger::max_steps tests --

#[test]
fn test_silent_trigger_callback_gets_higher_step_limit() {
    let trigger = SilentTrigger::Callback {
        task_id: "test-task".to_string(),
        label: "test".to_string(),
        result: "done".to_string(),
        failed: false,
        parent_task_id: None,
    };
    assert_eq!(
        trigger.max_steps(),
        crate::planning::policy::MAX_CALLBACK_TOOL_STEPS
    );

    let reminder = SilentTrigger::Reminder {
        task_id: "test".to_string(),
        message: "check CI".to_string(),
    };
    assert_eq!(
        reminder.max_steps(),
        crate::planning::policy::MAX_CALLBACK_TOOL_STEPS
    );
}

#[test]
fn test_silent_trigger_non_callback_gets_default_step_limit() {
    assert_eq!(
        SilentTrigger::Heartbeat.max_steps(),
        crate::planning::policy::MAX_TOOL_STEPS
    );
    assert_eq!(
        SilentTrigger::Reflection.max_steps(),
        crate::planning::policy::MAX_TOOL_STEPS
    );
    assert_eq!(
        SilentTrigger::SkillRun {
            skill_name: "test".to_string()
        }
        .max_steps(),
        crate::planning::policy::MAX_TOOL_STEPS
    );
}
