//! Integration tests for the agent eval harness.
//!
//! Run with: `cargo test -p mika-agent --test eval`

#[allow(dead_code)]
mod eval {
    pub mod assertions;
    pub mod calibration;
    pub mod harness;
    pub mod providers;
    pub mod scenarios;
    pub mod trace;

    // Harness eval multi-agents : N `agent_id` sur une base container partagée,
    // pour que la classe fan-out cesse d'être invisible (mika#2265).
    pub mod multi_agent;

    // Fixtures de processus réels partagées (mika#2265, AC2) : `spawn_live_child`
    // & co., extraites de test_pilot_silent_stall_reaper.rs.
    pub mod process_fixtures;

    // Porte transverse (mika#2265) : aucun fichier de tests/eval/ n'existe sans
    // être compilé. Elle se déclare elle-même.
    mod test_eval_modules_declared;

    // Golden dataset: 25 curated scenarios for end-to-end quality testing (#339)
    pub mod golden;

    // KG fixture helpers: shared seeding for KG eval scenarios (#740, #741)
    pub mod kg_fixtures;

    // KG self-knowledge: 7 scenarios for KG-backed self-knowledge (#740)
    pub mod kg_self_knowledge;

    // Grounding assertion helpers: shared for fabrication-detection scenarios (#741)
    pub mod grounding_assertions;

    // Grounding + fabrication regression: 5 scenarios from KG retrospective (#741)
    pub mod grounding_regressions;

    // Distribution Doctrine regression: public-promo suppression scenarios (mika#1814)
    pub mod doctrine_regressions;

    // KG provider evaluation matrix: provider comparison for extraction + resolution (#762)
    pub mod kg_provider_eval;

    // Per-skill eval scenarios — output-shape contracts validated through the
    // agent loop with synthetic skills + mock LLM (mika#879 Unit 1 onwards).
    pub mod skills;

    // v26->v27 KG migration invariant tests: coalesce per-agent data (#787)
    mod kg_v27_migration;

    // Porte 1 cascade probe (mika#1947) — gated #[ignore] +
    // MIKA_MANAGER_LOOP_RESISTANCE_TEST=1; pre-Phase-2-cut, not per-PR CI weight.
    mod manager_loop_resistance;

    mod test_auto_groom_dispatch;
    mod test_basic_conversation;
    mod test_callback_delivery_starvation;
    mod test_callback_milestone_advance;
    mod test_callback_terminal_action;
    mod test_callback_turn;
    mod test_ci_success_handler;
    mod test_completion_claim_guard;
    mod test_context_summary_inject;

    // mika#2305 — the scope that decided the window is emitted next to the count
    // it explains, and it is the one the identity declared. Its sibling below
    // asserts the window; this one asserts what the instrument says about it.
    mod test_context_scope_observability_2305;

    // mika#2425 — the per-tenant half. Its neighbour above covers the
    // identity-declared scope; this one covers a `customer_config` narrowing
    // reaching the real window, the refusal of a widening, and the unchanged
    // default. No source predicate can see a decision site that reads the right
    // field and simply never consults the database.
    mod test_context_history_per_tenant_2425;

    // mika#2295 briques 1 & 2 — the two window bounds, asserted on the window the
    // model actually received rather than on the predicates that compute it.
    mod test_context_window_budget_2295;

    // mika#1951 — the per-call isolation lever, on both cross-session channels,
    // with a negative control for each. The neighbours above cover the
    // identity-declared scope; this one covers a caller narrowing one turn.
    mod test_correction_message_classifier_guard;
    mod test_deadline_in_flight_llm_call;
    mod test_session_isolation_1951;

    // mika#2276 AC2 — la porte : `deadline dépassé ⇒ verdict posté`. Chaîne
    // complète, du vrai agent loop au POST, avec le poster injecté.
    mod test_deadline_verdict_2276;

    mod test_deferred_dispatch_idempotent_ack;

    // mika#2242 — fermer une PR umbrella sans merge dé-groome ses sous-tickets,
    // et rien ne nommait la cause. Le producteur estampille à la fermeture, le
    // lecteur relit au routage ; les deux contrôles négatifs (fermeture mergée,
    // premier grooming) sont les tests porteurs.
    mod test_degroom_attribution_2242;
    mod test_di_builders;
    mod test_dispatch_fired_at_stamped;
    mod test_dispatch_no_grooming_marker_guard;

    // mika#2405 — le fermeur des rows `manual` que la Delegation Rule fait
    // ouvrir avant un dispatch long-running : le chemin nominal avec son
    // contrôle négatif self_dev, la garde de vivacité sur ses deux versants, le
    // frère `completed`, et la non-régression de la faucheuse #871.
    mod test_dispatch_parent_settle_2405;
    mod test_dispatch_task_has_open_pr_guard;
    mod test_error_handling;

    // mika#2289 AC1 — la porte : `tour mort sur erreur LLM ⇒ verdict posté`.
    // Frère de `test_deadline_verdict_2276`, sur la branche `Err` de `run_agent`.
    mod test_error_verdict_2289;
    // mika#1784 — une image non lue est un fait dit. Les unités de
    // `image_disposition` ne verraient pas un `decide()` appelé au mauvais moment
    // dans la boucle ; ces tests lisent ce que le modèle a reçu.
    mod test_image_disposition_1784;
    // mika#1910 — le tour de continuation dit ce qu'il a produit. Les trois
    // valeurs que `response_chars` peut prendre y sont les trois populations que
    // R5 refuse de confondre : `Some(n)` produit, `Some(0)` LA classe, `None`
    // rien à mesurer.
    mod test_continuation_response_chars_1910;
    mod test_intent_precondition_guard;
    mod test_internal_tagging;
    mod test_kg_budget_757;

    // mika#2342 — le filet du site d'appel LLM principal : un appel qui dépasse
    // le pire cas déclaré du rail est coupé et laisse une trace, et un appel
    // lent mais borné ne l'est pas.
    mod test_llm_watchdog_2342;
    mod test_max_steps_continuation;
    // mika#2281 — la frontière de secret du canal MCP, mesurée sur un serveur
    // stdio factice : les noms traversent, la valeur non — et le contrôle
    // positif montre qu'elle traverse bel et bien par le canal FICHIER, qui
    // est le chemin réellement exposé.
    mod test_mcp_secret_boundary_2281;
    mod test_merge_identity_2248;
    // mika#2304 — le tour dit sous quel modèle il a tourné, et un modèle nommé
    // par l'appelant l'emporte sur la section `[llm]` d'une skill.
    mod test_model_attestation_2304;
    mod test_multi_agent_harness_witness;
    // mika#1883 — l'usage par tour est une SOMME : un tour à trois appels aux
    // usages distincts, la continuation max-steps ajoutée et non substituée, et
    // un tour non mesuré qui n'atteste rien plutôt qu'un zéro.
    mod test_multi_step;
    mod test_multi_turn_persistence;
    mod test_per_corpus_fairness_927;
    mod test_per_skill_provider_override;
    mod test_persistence_eval_guard;
    mod test_phantom_retry_guard;
    mod test_phantom_task_row_sweep;
    mod test_pilot_silent_stall_reaper;
    mod test_run_usage_aggregate_1883;

    // mika#2237 — le mapping verdict → flag de `gh pr review`, vérifié avant le
    // sous-processus. Chemin de production déterministe (MockLlmProvider) pour
    // les deux directions, l'échappatoire D4 et ses trois contrôles négatifs.
    mod test_pr_review_flag_coherence_2237;

    // Le contrôle positif sur processus réel du reaper D1 (mika#2272) : row
    // `pending` sémée par le chemin de production, pilote authentiquement
    // vivant, harness multi-agents pour l'attribution.
    mod test_pr_review_idempotency;

    // mika#2455 — un `pass` ne peut plus affirmer ce qu'un check requis rouge
    // contredit : les deux têtes mesurées, l'issue laissée ouverte sous
    // mika#2237, et les six abstentions nommées.
    mod test_qa_ci_coherence_2455;

    // mika#2334 — la revue ne dépend plus d'un `pull_request.opened` que rien
    // ne rejoue : l'incident rejoué, plus les trois faits de câblage.
    mod test_qa_review_reconcile_2334;

    // mika#2347 — le ledger devient décisionnel : cooldown et budget keyés
    // (dépôt, PR, head SHA), plus l'invariant « un tour par agent » épinglé.
    mod test_qa_review_reconcile_2347;
    // mika#2323 — un `labeled ready` reçu laisse toujours une trace, et cette
    // trace nomme la porte qui a décidé (il n'y a aucun filtre acteur).
    mod test_ready_label_attribution_2323;
    mod test_ready_label_blocked_skip;
    mod test_ready_label_grooming_guard;

    // mika#2279 — un `labeled ready` répété sur un ticket dont le pilote est vif
    // est un NO-OP : processus réel, topologie parent/enfant de production.
    mod test_ready_label_live_pilot_noop_2279;

    // mika#2279 — le terme sur lequel repose le filtre 4b d'`auto_pull`, mesuré
    // sur un processus réellement tué et récolté (la décision elle-même est
    // épinglée in-crate, là où vit son classifieur).
    mod test_auto_pull_live_pilot_filter_2279;
    mod test_real_provider_matrix;

    // mika#2337 — un trigger enregistré a un destinataire (garde de classe),
    // et une mort par trigger inconnu ne gèle plus la récurrence 24 h.
    mod test_recurring_trigger_wiring_2337;

    // La porte de mika#2277 : le prédicat de kill est une conjonction sur les
    // trois surfaces du pilote. Contrôle positif + N1..N4, même suite.
    mod test_reaper_liveness_all_surfaces_2277;
    mod test_reaper_reaps_live_pending_pilot_2272;
    mod test_request_wellformedness;
    mod test_required_tools_gate;
    mod test_schema_divergence;
    mod test_self_knowledge_kg;
    mod test_supersede_kills_live_pilot;
    mod test_task_not_found_retry;
    mod test_tool_call_secret_redaction;
    mod test_tool_call_stream_emission;
    mod test_tool_calling;
    mod test_tracking_row_supersede;
    mod test_tracking_row_upstream_close;
    mod test_unauthorized_webhook_dispatch_tool_boundary;
    mod test_verdict_handler;
    mod test_webhook_no_unauthorized_dispatch_guard;
    mod test_webhook_queue;
    mod test_webhook_zero_tools_guard;

    // mika#2517 — un tour du domaine Webhook Fallthrough ne se voit pas servir
    // `create_task`. Les deux contrôles négatifs (ready-label, tour ordinaire)
    // sont porteurs : le test principal seul serait satisfait par un filtre qui
    // retire l'outil à TOUS les tours, c'est-à-dire par un correctif qui casse
    // la boucle en ayant l'air de fermer le ticket.
    mod test_webhook_fallthrough_no_task_2517;

    // qa-review skill-scoped run_gh validator wiring test (mika#1196)
    mod test_qa_review_run_gh_scope_validator;

    // mika#2355 — le callback de build QA poste son verdict : B2 (carve-out
    // #870 + framing) et B3 (garde positive) sur le chemin silencieux réel.
    mod test_qa_build_callback_verdict_2355;

    // mika#1952 — le schéma SERVI en mode réflexion déclare `evidence` requis,
    // les autres modes sont inchangés, et la garde d'exécution reste la seule
    // barrière dure (`"evidence": ""` satisfait `required` et échoue la garde).
    mod test_reflection_evidence_contract_1952;

    // mika#2368 — le filet moteur : le signal levé sur les DEUX sites de sortie
    // EndTurn du tour silencieux, et le registre anti-double-post qui l'atteint.
    mod test_qa_callback_verdict_net_2368;

    // mika#2515 — le signal sort des CINQ sorties atteignables, pas de deux : le
    // « Force EndTurn » de `send_message` (P0, vu rouge avant U1e) et les deux
    // coupures (deadline, max-steps), avec leurs contrôles négatifs.
    mod test_qa_cut_off_verdict_2515;

    // mika#2358 — la garde 5e sur le chemin de production : une promesse de
    // fréquence sans acteur est refusée, l'aveu d'incapacité passe, et le
    // budget d'un seul re-prompt tient.
    mod test_unactioned_frequency_promise_2358;

    // Self-dev-callback engine consistency: documented callback-handler branches
    // produce tool calls the engine accepts or defers (mika#806).
    mod test_self_dev_callback_engine_consistency;

    // Send-message turn boundary guard: prevents write tools after
    // send_message in conversation mode (#771).
    mod test_send_message_boundary;

    // Un `send_message` échoué ne peut pas se clore en silence : guard 6f,
    // prédicat structurel, et le rejeu du 2026-09-01 (mika#2136).
    mod test_undelivered_send_2136;

    // Multi-agent corpus parity: regression guard for #1155 search_content gap
    mod kg_multi_agent_corpus_parity;

    // Compact provider gate: MikaModel request shape regression (mika#1491)
    mod test_compact_provider_gate;
}
