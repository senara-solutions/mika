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

    // mika#2260 — l'attribution du fan-out `check_suite` est mesurée sur une base
    // container partagée : un seul agent évalue, dans les deux patrons (ordre et
    // course).
    mod test_ci_success_fanout_2260;
    mod test_completion_claim_guard;

    // mika#1748 — la cérémonie de handsoff au plafond de contexte, lue dans le
    // prompt système que le modèle a reçu, avec son contrôle négatif à un
    // message près. Les tests unitaires du prédicat et du builder resteraient
    // verts si l'entonnoir ne comptait jamais les messages.
    mod test_compaction_handsoff_1748;

    // mika#2805 — le vol 2 du même entonnoir : la déposition est servie au
    // successeur, le reçu est posé dans le tour servi, et le second tour ne la
    // ressert pas. Le troisième test est le contrôle négatif de la porte D2 —
    // le déclencheur est la condensation passée, jamais la présence d'un
    // fichier.
    mod test_compaction_handsoff_ritual_2805;

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

    // mika#2884 — la ligne d'attribution porte la taille du prompt système
    // (AC1), et c'est la même valeur que celle du voisin qui la rapportait déjà
    // (AC2). Un unitaire sur l'émetteur resterait vert si le site d'appel
    // passait `0` ou la longueur d'un autre `String` : seul un tour réel, lu sur
    // l'événement émis ET sur la requête capturée, relie les deux bouts.
    mod test_context_window_system_prompt_bytes_2884;

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

    // mika#2784 — ce qu'un agent fail-closed reçoit réellement comme outils.
    // Les unités de `prompt::fail_closed_identity` lisent les champs de la
    // structure ; elles resteraient vertes si la posture n'était jamais
    // consultée au moment de publier le tableau. Frère de
    // `test_mika_arch_tool_allowlist_2748` sur la posture dégradée, contrôle
    // négatif compris.
    mod test_fail_closed_tool_surface_2784;

    // mika#2750 — la faucheuse de parents orphelins balaie aussi la classe
    // `groom` : le contrôle positif (un groom orphelin atteint un état terminal
    // nommé), le contrôle négatif (pilote vivant ⇒ rien ne bouge) et la
    // non-régression mot-pour-mot du format de fil de la classe `implement`.
    mod test_groom_parent_reap_2750;
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
    // mika#2877 — à la coupure au plafond de steps, le décompte par outil du
    // tour est exact (AC1), et un tour qui ne s'épuise pas ne l'émet pas (AC2) :
    // c'est la paire qui est le livrable, un contrôle d'absence seul n'attestant
    // rien.
    mod test_max_steps_tool_census_2877;
    // mika#2281 — la frontière de secret du canal MCP, mesurée sur un serveur
    // stdio factice : les noms traversent, la valeur non — et le contrôle
    // positif montre qu'elle traverse bel et bien par le canal FICHIER, qui
    // est le chemin réellement exposé.
    mod test_mcp_secret_boundary_2281;
    // mika#2814 — l'équivalent L2 côté MCP : un outil dont le serveur déclare
    // `testimony` n'est jamais PUBLIÉ au modèle, qui ne peut donc plus le
    // proposer. Mesuré sur les octets soumis, avec le contrôle positif
    // `operational` qui montre que les mêmes noms traversent sans le grade.
    mod test_mcp_testimony_non_publication_2814;
    mod test_merge_identity_2248;
    // mika#2748 — le câblage de l'allowlist d'outils, vu sur le tableau que le
    // modèle a réellement reçu : ni le prédicat (D8) ni le chargement TOML (D9)
    // ne rougissent si les sites de production ne passent jamais le champ.
    mod test_mika_arch_tool_allowlist_2748;
    // mika#2304 — le tour dit sous quel modèle il a tourné, et un modèle nommé
    // par l'appelant l'emporte sur la section `[llm]` d'une skill.
    mod test_model_attestation_2304;
    mod test_multi_agent_harness_witness;
    // mika#1883 — l'usage par tour est une SOMME : un tour à trois appels aux
    // usages distincts, la continuation max-steps ajoutée et non substituée, et
    // un tour non mesuré qui n'atteste rien plutôt qu'un zéro.
    mod test_multi_step;
    mod test_multi_turn_persistence;

    // mika#2506 — le geste déterministe d'itération, sur son chemin de
    // production : le numéro d'ISSUE qui voyage (avec le numéro de PR en
    // contrôle négatif), la tâche auto-descriptive dès sa création, le créneau
    // occupé qui REFUSE au lieu de différer, et les quatre refus vus là où la
    // forge est réellement sollicitée.
    mod test_operator_iterate_2506;
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

    // mika#2519 — l'exemption de plan dependabot existait deux fois et ne tenait
    // pas : le `block[pipeline]` que Step 1.6 rend inatteignable est refusé
    // avant le sous-processus, et un `pass` sur un saut de MAJEURE exige une
    // assertion de sites d'appel. Les deux branches, leurs quatre contrôles
    // négatifs et les abstentions nommées.
    mod test_dependabot_verdict_coherence_2519;

    // mika#2565 — un bump Rust se vérifie en compilant, jamais en affirmant. La
    // branche B3 lit un FAIT MOTEUR (`tool_calls`) plutôt qu'une phrase du
    // corps : #2561 portait une `API-SURFACE:` réelle et insuffisante, #2560
    // était hors de B2 par `NoMajorJump`. Le contrôle positif (un build en base
    // laisse passer) et les deux abstentions nommées.
    mod test_dependabot_build_evidence_2565;

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

    // mika#2456 — `enabled = false` niveau-agent éteint les récurrentes : la
    // porte mord, annule la ligne préexistante, ne ressuscite pas, reste
    // réversible, et `mika tasks rearm` refuse nommément. Avec son contrôle
    // négatif à un octet d'écart.
    mod test_agent_disabled_recurring_gate_2456;

    // La porte de mika#2277 : le prédicat de kill est une conjonction sur les
    // trois surfaces du pilote. Contrôle positif + N1..N4, même suite.
    mod test_kg_zero_budget_1833;
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

    // mika#2573 — le frère du précédent, sur l'outil voisin : un tour du domaine
    // Webhook Fallthrough se voit REFUSER les deux verbes `run_gh` qui créent du
    // travail (`issue create`, `issue edit --add-label ready`), et garde ses
    // lectures. Les quatre contrôles négatifs sont porteurs : V1/V2 seuls
    // seraient satisfaits par une garde qui bloque `run_gh` sur tout tour
    // Fallthrough, c'est-à-dire qui casse le geste de vérification que la SCOPE
    // RULE du prompt prescrit littéralement.
    mod test_fallthrough_run_gh_refused_2573;

    // mika#2653 — second cas mesuré de la même famille, et le plus coûteux : un
    // tour ouvert par un événement PR / check-suite n'annule pas une tâche dont
    // un pilote est vif (71 tours jetés le 2026-10-02). Les trois contrôles sont
    // porteurs : V6b (tour de conversation) sépare « la garde décide » de « la
    // garde bloque l'annulation » — et retirer à l'opérateur son geste
    // d'annulation est pire que le défaut, puisqu'il n'a pas de contournement ;
    // V6c (tour webhook PR sans pilote) est le contrôle POSITIF, sans quoi zéro
    // refus est indistinguable d'une garde qui ne tourne pas ; V6d épingle le
    // fail-closed sur l'illisible, qui est l'arbitrage inverse de celui de
    // `live_pilot` pour ses deux appelants.
    mod test_cancel_task_live_pilot_2653;

    // mika#2654 — troisième de la famille, ajouté par le recensement AC4 de
    // mika#2649 : un tour ouvert par un événement PR / check-suite ne force pas
    // la promotion d'un wrapper différé, donc ne choisit pas quel dispatch prend
    // le créneau de sa classe. Les unitaires posent
    // `ctx.is_webhook_pr_event_turn` à la main, donc ce fichier est le SEUL à
    // attester le câblage au site de conversation — c'est la mutation M4 du plan.
    // Les deux contrôles sont porteurs : le tour de conversation sépare « la
    // garde décide » de « la garde bloque la promotion forcée » (~42 appels
    // mesurés hors canal github), et le tour `[GitHub] Issue …` épingle que le
    // domaine Webhook Fallthrough reste hors périmètre PAR DÉCISION.
    mod test_promote_deferred_webhook_pr_2654;

    // qa-review skill-scoped run_gh validator wiring test (mika#1196)
    mod test_qa_review_run_gh_scope_validator;

    // mika#2355 — le callback de build QA poste son verdict : B2 (carve-out
    // #870 + framing) et B3 (garde positive) sur le chemin silencieux réel.
    mod test_qa_build_callback_verdict_2355;

    // mika#2727 — l'AUTRE bout du même tour : le tour d'OUVERTURE de revue,
    // ouvert par `[GitHub] PR synchronize: …` en mode conversation, qui finissait
    // en `EndTurn` avec son verdict en texte et rien sur GitHub. AC3a (la garde
    // tire) et AC3b (contrôle positif : `pr review` ne re-prompte pas) dans le
    // même fichier, plus le caractère à un coup du re-prompt.
    mod test_qa_review_submission_2727;

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

    // mika#2926 — le budget de pas d'un tour de revue : le corpus rejoué de
    // `callback-0f07c520` pose son verdict sous le plafond de mika-qa, la MÊME
    // séquence l'épuise sur un autre agent (c'est ce qui tient AC3), et le WARN
    // de coupure annonce le plafond SERVI et non celui du déclencheur.
    mod test_qa_step_budget_2926;

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

    // Fenêtre de transmission au changement de modèle : le signal posé atteint
    // le prompt conversationnel, et son contrôle négatif (mika#1747 vol 1)
    mod test_model_transmission_1747;

    // La moitié entrante du même rituel : la lettre du prédécesseur atteint le
    // prompt, une seule fois — le second tour sur la même graine ne la rend
    // plus (mika#2807 vol 2)
    mod test_incoming_transmission_2807;

    // Le site P0 amiral atteint le transport en P0, bout en bout à travers
    // `run_agent`, et son contrôle négatif sur le prédicat (mika#1381 vol 3)
    mod test_dispatch_stall_p0_1381;

    // L'autre moitié de la même propriété : sur le domaine Webhook Fallthrough
    // l'accusé P2 ne part à personne, la garde reste armée hors du domaine, et
    // un accusé passé par l'outil `send_message` arrive sans étage — donc la
    // suppression est une propriété du garde, jamais du routage (mika#1381 vol 4)
    mod test_p2_ack_suppression_1381;

    // La classe `fabrication-detected` de mika#1381 a enfin un site, et il est
    // attesté au transport : la récidive de la garde `assert_grounded` part en
    // P0 porte ouverte, et rien ne part porte fermée — le défaut livré
    // (mika#2867)
    mod test_fabrication_detected_p0_2867;

    // Le second site de la même classe, et le créneau devenu typé : la récidive
    // de la garde `false_local_hosting_claim` (5d) part en P0 porte ouverte,
    // rien ne part porte fermée, et la revérification à l'acceptation rejoue le
    // prédicat DU SITE qui a déposé — le fixture ne porte aucune forme
    // d'`assert_grounded`, donc le prédicat de l'autre site ne peut pas tenir
    // ce test par accident (mika#2872)
    mod test_fabrication_detected_p0_2872;

    // Le troisième site de la même classe, et le premier dont la frontière de
    // classe passe à l'INTÉRIEUR d'un garde : la récidive de 5e
    // (`unactioned_frequency_promise`) part en P0 sur sa seule moitié « état
    // accompli », et sa moitié « engagement sur l'avenir » ne lève rien — porte
    // armée dans les deux cas, donc seule la nature de l'assertion décide
    // (mika#2879)
    mod test_fabrication_detected_p0_2879;

    // Le verrou de CLASSE de la même famille, enfin attesté : un tour qui viole
    // 5d + 5e + assert_grounded à la fois n'émet qu'une P0, et c'est celle du
    // premier écrivain de la chaîne ; et un engagement sur l'avenir ne prend pas
    // le créneau du site aval. Les neuf fixtures mono-site des vols 1 à 3 ne
    // pouvaient pas le voir — c'est leur R6/R7 resté vert (mika#2879, vol 4/4)
    mod test_fabrication_detected_p0_2879_collision;
}
