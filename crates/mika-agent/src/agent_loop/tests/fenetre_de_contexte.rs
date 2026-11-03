//! La fenêtre de conversation : attribution, recensement, plafond d'octets.
//!
//! L'instrument d'attribution de la fenêtre (mika#2295 brique 0), le
//! recensement par outil et l'égalité qui en fait une population avec
//! `tool_defs_bytes` (mika#2783), le champ qui rapporte la portée dont il
//! vient (mika#2305) et le plafond d'octets (mika#2295 brique 2).
//!
//! La moitié *scan de source* de mika#2305 — « la portée n'a qu'un lecteur
//! décisionnel » — vit dans `gardes_de_source.rs`, avec les autres scans.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// ===========================================================================
// mika#2295 brique 0 — context-window attribution instrument
// ===========================================================================

fn window_msg(session_id: &str, content: &str, created_at: &str) -> crate::db::SessionMessage {
    crate::db::SessionMessage {
        id: 0,
        session_id: session_id.to_string(),
        agent_id: "mika-arch".to_string(),
        role: "user".to_string(),
        content: content.to_string(),
        channel_type: "cli".to_string(),
        metadata: None,
        trace_id: None,
        created_at: created_at.to_string(),
        internal: false,
    }
}

fn at(secs_ago: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() - chrono::Duration::seconds(secs_ago)
}

#[test]
fn mika2295_history_bytes_excludes_the_turn_s_own_user_message() {
    // The whole point of the split: `history_bytes` must answer "what does
    // this request carry BEYOND the question?". Folding the turn's own user
    // message into it would make a large plan look like a large history and
    // send the correction at the wrong component.
    let now = chrono::Utc::now();
    let history = vec![
        window_msg("s1", "aaaa", &crate::timestamp::format(&at(60))),
        window_msg("s1", "bbbbbb", &crate::timestamp::format(&at(30))),
        window_msg(
            "s1",
            "the plan under review",
            &crate::timestamp::format(&at(0)),
        ),
    ];

    let f = build_context_window_fields(&history, prompt::HistoryScope::Agent, &[], 0, 0, now);

    assert_eq!(f.message_count, 3);
    assert_eq!(f.history_bytes, 4 + 6);
    assert_eq!(f.user_message_bytes, "the plan under review".len());
}

#[test]
fn mika2295_distinct_sessions_is_the_cross_ticket_contamination_detector() {
    // The window is filtered on `m.agent_id`, not `m.session_id`, so it
    // crosses sessions and therefore tickets. This field is what turns that
    // property from a code reading into a measurement.
    let now = chrono::Utc::now();
    let ts = crate::timestamp::format(&at(10));

    let one_ticket = vec![
        window_msg("s1", "a", &ts),
        window_msg("s1", "b", &ts),
        window_msg("s1", "c", &ts),
    ];
    assert_eq!(
        build_context_window_fields(&one_ticket, prompt::HistoryScope::Agent, &[], 0, 0, now)
            .distinct_sessions,
        1,
        "a window confined to one session must report exactly 1"
    );

    let three_tickets = vec![
        window_msg("ticket-a", "plan A", &ts),
        window_msg("ticket-b", "plan B", &ts),
        window_msg("ticket-c", "plan C", &ts),
        window_msg("ticket-a", "review A", &ts),
    ];
    assert_eq!(
        build_context_window_fields(&three_tickets, prompt::HistoryScope::Agent, &[], 0, 0, now)
            .distinct_sessions,
        3,
        "three distinct sessions in the window is the contamination signal"
    );
}

#[test]
fn mika2295_oldest_age_is_taken_from_the_first_message() {
    // `rebuild_context` returns oldest-first (the SQL sorts DESC then the
    // loader reverses), so the age is read off `first()`. If that ordering
    // ever flips, this assertion is what says so.
    let now = chrono::Utc::now();
    let history = vec![
        window_msg("s1", "old", &crate::timestamp::format(&at(3600))),
        window_msg("s1", "recent", &crate::timestamp::format(&at(5))),
    ];

    let f = build_context_window_fields(&history, prompt::HistoryScope::Agent, &[], 0, 0, now);
    assert!(
        (3595..=3605).contains(&f.oldest_age_secs),
        "expected ~3600s, got {}",
        f.oldest_age_secs
    );
}

#[test]
fn mika2295_unreadable_or_future_timestamp_reports_zero_never_a_negative() {
    // A negative age would read as a clock claim the instrument cannot back.
    // Zero is the honest floor, and it is the rule `timestamp::is_older_than`
    // already applies to the same class of input.
    let now = chrono::Utc::now();

    let garbled = vec![window_msg("s1", "x", "not-a-timestamp")];
    assert_eq!(
        build_context_window_fields(&garbled, prompt::HistoryScope::Agent, &[], 0, 0, now)
            .oldest_age_secs,
        0
    );

    let from_the_future = vec![window_msg(
        "s1",
        "x",
        &crate::timestamp::format(&(now + chrono::Duration::seconds(600))),
    )];
    assert_eq!(
        build_context_window_fields(
            &from_the_future,
            prompt::HistoryScope::Agent,
            &[],
            0,
            0,
            now
        )
        .oldest_age_secs,
        0
    );
}

#[test]
fn mika2295_empty_window_reports_zeroes_rather_than_panicking() {
    // A fresh session assembles an empty window. The instrument must survive
    // it: an attribution event that panics on the first turn of a session
    // would take the turn down with it.
    let f = build_context_window_fields(
        &[],
        prompt::HistoryScope::Agent,
        &[],
        0,
        0,
        chrono::Utc::now(),
    );

    assert_eq!(f.message_count, 0);
    assert_eq!(f.history_bytes, 0);
    assert_eq!(f.user_message_bytes, 0);
    assert_eq!(f.distinct_sessions, 0);
    assert_eq!(f.oldest_age_secs, 0);
}

#[test]
fn mika2295_single_message_window_is_all_user_message_and_no_history() {
    // The first turn of a session: the only message IS the question, so the
    // history it carries is zero. An off-by-one in `split_last` would show up
    // here as the question being counted as its own history.
    let history = vec![window_msg(
        "s1",
        "review this plan",
        &crate::timestamp::format(&at(1)),
    )];

    let f = build_context_window_fields(
        &history,
        prompt::HistoryScope::Agent,
        &[],
        0,
        0,
        chrono::Utc::now(),
    );

    assert_eq!(f.history_bytes, 0);
    assert_eq!(f.user_message_bytes, "review this plan".len());
}

#[test]
fn mika2295_tool_defs_bytes_is_the_serialized_size() {
    // Tool definitions are the third component of the request and grow
    // independently of the window. Reporting them here is what lets the
    // analyzer subtract a known quantity instead of attributing its growth to
    // the history by elimination.
    let defs = full_tool_set();
    let expected = serde_json::to_string(&defs).unwrap().len();

    let f = build_context_window_fields(
        &[],
        prompt::HistoryScope::Agent,
        &defs,
        0,
        0,
        chrono::Utc::now(),
    );

    assert_eq!(f.tool_defs_bytes, expected);
    assert!(f.tool_defs_bytes > 0);
}

#[test]
fn mika2295_truncation_counts_are_passed_through_not_invented() {
    // They are parameters rather than literals so the byte ceiling could be
    // lifted without reshaping the event an analyzer has already been written
    // against. Since brique 2, the production call site passes real counts.
    let f = build_context_window_fields(
        &[],
        prompt::HistoryScope::Agent,
        &[],
        7,
        4096,
        chrono::Utc::now(),
    );

    assert_eq!(f.truncated_messages, 7);
    assert_eq!(f.truncated_bytes, 4096);
}

// ===========================================================================
// mika#2783 — the per-tool census, and the equality that makes it one
// population with `tool_defs_bytes` rather than two
// ===========================================================================

/// **The load-bearing equality, on the REAL registry.**
///
/// `tool_defs_bytes` and the census must measure one array, not two
/// populations under one reading (class mika#2205). A synthetic fixture proves
/// the arithmetic; only the registry proves it against the definitions
/// production actually serves — the same authority reason
/// `tools::tests::mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste`
/// reads `default_tools()` rather than a hand-written list.
#[test]
fn mika2783_la_somme_par_outil_plus_le_cadrage_egale_le_total() {
    let registry = crate::tools::default_tools();
    let defs = registry.definitions();
    let census = tool_defs_census(defs);

    assert_eq!(
        census.per_tool_sum + json_array_framing_bytes(census.count),
        tool_defs_byte_weight(defs),
        "the census and tool_defs_bytes disagree on the real registry"
    );
    // Anti-vacuity: an empty registry would make the equality hold at 2 == 2
    // and assert nothing about any tool.
    assert!(census.count > 1, "the real registry serves no tools");
    assert_eq!(census.per_tool.len(), census.count, "duplicate tool name");
}

/// The same equality on a three-definition array — **this is the test that
/// makes risk R1 of the plan a red `cargo test` instead of a WARN in
/// production.** Three is the smallest count that exercises more than one
/// comma.
#[test]
fn mika2783_la_somme_par_outil_plus_le_cadrage_egale_le_total_sur_un_tableau_synthetique_de_trois_definitions()
 {
    let defs = vec![tool_def("send_message"), tool_def("run_gh"), tool_def("x")];
    let census = tool_defs_census(&defs);

    assert_eq!(census.count, 3);
    assert_eq!(json_array_framing_bytes(3), 4, "two brackets, two commas");
    assert_eq!(
        census.per_tool_sum + json_array_framing_bytes(census.count),
        tool_defs_byte_weight(&defs)
    );
}

/// An empty array is `[]` — two bytes of frame and nothing else, and the
/// saturating subtraction is what stops `0 - 1` wrapping to `usize::MAX`.
///
/// The map is **empty**, which is the `{}` the event degrades to and never a
/// set of zeroed keys: no tool was served, so no tool has a cost.
#[test]
fn mika2783_un_tableau_vide_rend_une_carte_vide_et_un_cadrage_de_deux() {
    let census = tool_defs_census(&[]);

    assert_eq!(census.count, 0);
    assert!(census.per_tool.is_empty());
    assert_eq!(census.per_tool_sum, 0);
    assert_eq!(json_array_framing_bytes(0), 2);
    assert_eq!(
        census.per_tool_sum + json_array_framing_bytes(census.count),
        tool_defs_byte_weight(&[]),
        "`[]` is two bytes"
    );
}

/// One definition carries no comma — the `n − 1` term at its boundary.
#[test]
fn mika2783_une_definition_unique_na_pas_de_virgule() {
    let defs = vec![tool_def("send_message")];
    let census = tool_defs_census(&defs);

    assert_eq!(json_array_framing_bytes(1), 2);
    assert_eq!(
        census.per_tool_sum + json_array_framing_bytes(census.count),
        tool_defs_byte_weight(&defs)
    );
}

/// **Absent is never zero (mika#2331).** A tool the turn did not serve has no
/// key at all; a `0` would read as "served, and free", which is the one
/// reading that would make the instrument lie to the arbitration it exists
/// for.
#[test]
fn mika2783_un_outil_non_servi_na_pas_de_cle() {
    let census = tool_defs_census(&[tool_def("send_message")]);

    assert!(census.per_tool.contains_key("send_message"));
    assert!(
        !census.per_tool.contains_key("run_gh"),
        "a non-served tool must have no key, not a zero"
    );
    assert_eq!(census.per_tool.get("run_gh"), None);
}

/// Two definitions under one name are **summed**, so the equality holds
/// whatever the array contains, and the duplication is reported rather than
/// swallowed.
///
/// Reachable: the MCP append at the window-assembly site does not deduplicate
/// against the builtin and skill names. Note `count` stays at the number of
/// *definitions* — `per_tool.len()` would be `1` here, and the framing
/// arithmetic is a function of the former.
#[test]
fn mika2783_un_nom_en_doublon_est_somme_et_legalite_tient() {
    let defs = vec![tool_def("run_gh"), tool_def("run_gh")];
    let census = tool_defs_census(&defs);

    assert_eq!(census.count, 2);
    assert_eq!(census.per_tool.len(), 1);
    assert_eq!(census.duplicates, vec![("run_gh".to_string(), 2)]);
    assert_eq!(
        census.per_tool["run_gh"], census.per_tool_sum,
        "the two occurrences are summed under one key"
    );
    assert_eq!(
        census.per_tool_sum + json_array_framing_bytes(census.count),
        tool_defs_byte_weight(&defs)
    );
}

/// The map is ordered, because the line is read by eye and diffed between two
/// turns. A `HashMap` would reshuffle the field on every emission and make two
/// readings of an unchanged array look like a change.
#[test]
fn mika2783_la_carte_est_ordonnee() {
    let defs = vec![tool_def("zeta"), tool_def("alpha"), tool_def("mid")];
    let census = tool_defs_census(&defs);

    let keys: Vec<&str> = census.per_tool.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["alpha", "mid", "zeta"]);

    // And the serialized form an operator reads carries that same order.
    let json = serde_json::to_string(&census.per_tool).unwrap();
    let alpha = json.find("alpha").unwrap();
    let mid = json.find("mid").unwrap();
    let zeta = json.find("zeta").unwrap();
    assert!(
        alpha < mid && mid < zeta,
        "serialized map is unordered: {json}"
    );
}

// ===========================================================================
// mika#2305 — the scope travels with the count it explains
// ===========================================================================

/// **T3 (pure half)** — the field reports the scope it was handed, for both
/// variants.
///
/// The values are a wire format: an operator `jq`s on them and `GROUP BY`s
/// them, so a rename silently splits one population in two. Pinning the two
/// literals here is what makes that a compile-and-test failure rather than a
/// dashboard that quietly stops matching.
#[test]
fn mika2305_the_field_reports_the_scope_it_was_given() {
    let now = chrono::Utc::now();
    let ts = crate::timestamp::format(&at(10));
    let history = vec![window_msg("s1", "a", &ts)];

    assert_eq!(
        build_context_window_fields(&history, prompt::HistoryScope::Session, &[], 0, 0, now)
            .history_scope,
        "session"
    );
    assert_eq!(
        build_context_window_fields(&history, prompt::HistoryScope::Agent, &[], 0, 0, now)
            .history_scope,
        "agent"
    );
}

/// **T4** — the label match carries no `_ =>` arm.
///
/// Rust already forces exhaustiveness; what it does **not** force is that a
/// future variant be *decided* rather than absorbed. A `_ => "agent"` would
/// compile, keep every assertion above green, and label the new scope with
/// somebody else's name — which is the one failure this field exists to
/// prevent. Hence a source scan: the defect makes no covered decision wrong.
#[test]
fn mika2305_the_label_match_has_no_wildcard_arm() {
    // Read through the canonical reader rather than `fs::read_to_string`
    // (mika#3023). This guard asserts an **absence** inside the function's body,
    // the one class that stays green while having stopped proving anything: a
    // fixture quoting `fn history_scope_label(` ahead of the real definition
    // would hand `find` someone else's body, and the assertion would pass on it.
    // The canonical reader hands over the production half only, so the fixture
    // cannot be the match — and the `.expect` below keeps the anchor loud.
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));

    let sig = "fn history_scope_label(";
    let start = src
        .find(sig)
        .expect("history_scope_label must exist in agent_loop/mod.rs");
    let rest = &src[start..];
    let end = rest.find("\n}\n").expect("the function must be closed");
    let body = &rest[..end];

    assert!(
        !body.contains("_ =>"),
        "mika#2305 — `history_scope_label` grew a wildcard arm. A future \
             `HistoryScope` variant would then inherit an existing label, and \
             `context_window_assembled` would report a scope that is not the one \
             in force. Name the new variant explicitly instead.\nbody was:\n{body}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// mika#2474 — the tail of the brief-size distribution.
// ─────────────────────────────────────────────────────────────────────

/// Build a window whose last message — the turn's user message, i.e. the plan
/// on an architect pass — weighs exactly `bytes`.
fn window_with_brief(bytes: usize) -> ContextWindowFields {
    let ts = crate::timestamp::format(&at(10));
    let history = vec![
        window_msg("s1", "older history", &ts),
        window_msg("s1", &"p".repeat(bytes), &ts),
    ];
    build_context_window_fields(
        &history,
        prompt::HistoryScope::Session,
        &[],
        0,
        0,
        chrono::Utc::now(),
    )
}

/// The threshold is what decides, **and the boundary is exclusive**.
///
/// The `==` case is the negative control that separates "the threshold
/// decides" from "anything non-trivial is reported": at exactly p90 a brief is
/// still inside the healthy distribution the threshold was calibrated on.
#[test]
fn mika2474_the_verdict_fires_only_above_the_threshold() {
    assert_eq!(brief_size_overrun_verdict(48_001, 48_000), Some(48_000));
    assert_eq!(brief_size_overrun_verdict(48_000, 48_000), None);
    assert_eq!(brief_size_overrun_verdict(1_260, 48_000), None);

    // The resolved threshold is reported, not the default: a 50 KB brief read
    // against 48 000 and the same brief read against 20 000 are two different
    // facts, and the line has to say which.
    assert_eq!(brief_size_overrun_verdict(50_000, 20_000), Some(20_000));

    // A non-representable threshold reports nothing — silence is the fail-safe
    // direction for an alert. Unreachable in production (the accessor
    // guarantees a positive `i64`), asserted rather than assumed.
    assert_eq!(brief_size_overrun_verdict(usize::MAX, -1), None);
}

/// The crossing writes one audit row, and `after_value` carries the bytes and
/// nothing else — it is what the operator averages.
#[tokio::test]
async fn mika2474_an_oversized_brief_is_reported_and_audited() {
    let db = test_async_db();
    let fields = window_with_brief(72_349); // p99 of September 2026.

    report_brief_size_overrun(
        &db,
        "test-session",
        "trace-2474",
        "conversation",
        &fields,
        16_562, // the `mika-arch-groom-ticket` prompt, measured.
        48_000,
    )
    .await;

    let rows = db.get_audit_events("test-session").await.unwrap();
    let overruns: Vec<_> = rows
        .iter()
        .filter(|r| r.tool_name == BRIEF_SIZE_OVERRUN_EVENT)
        .collect();

    assert_eq!(overruns.len(), 1, "exactly one row per crossing");
    assert_eq!(overruns[0].target_key, format!("agent:{}", db.agent_id));
    assert_eq!(
        overruns[0].after_value.as_deref(),
        Some("72349"),
        "after_value is the byte count alone — the `pilot_cost_overrun` motif"
    );
    assert_eq!(
        overruns[0].before_value, None,
        "a size has no prior state to name"
    );
    assert_eq!(overruns[0].trace_id.as_deref(), Some("trace-2474"));

    // The reasoning carries the other three components so the row can be read
    // without a second query.
    let reasoning = overruns[0].reasoning.as_deref().unwrap_or_default();
    for expected in ["trace_id=trace-2474", "system=16562", "threshold=48000"] {
        assert!(
            reasoning.contains(expected),
            "reasoning must carry {expected}; got {reasoning}"
        );
    }
}

/// **The negative control, and without it "the threshold decides" is
/// indistinguishable from "the threshold always emits".**
#[tokio::test]
async fn mika2474_a_brief_under_the_threshold_emits_nothing() {
    let db = test_async_db();
    let fields = window_with_brief(29_414); // the measured median.

    report_brief_size_overrun(
        &db,
        "test-session",
        "trace-nominal",
        "conversation",
        &fields,
        16_562,
        48_000,
    )
    .await;

    let rows = db.get_audit_events("test-session").await.unwrap();
    assert!(
        rows.iter().all(|r| r.tool_name != BRIEF_SIZE_OVERRUN_EVENT),
        "a median brief must write no row: this is a threshold, not a per-turn event"
    );
}

// ===========================================================================
// mika#2295 brique 2 — the byte ceiling
// ===========================================================================

/// A window of `n` history messages of `bytes` each, plus the turn's own
/// user message last — the shape `rebuild_context` returns.
fn window_with_history(sizes: &[usize], user_message: &str) -> Vec<crate::db::SessionMessage> {
    let mut out: Vec<crate::db::SessionMessage> = sizes
        .iter()
        .enumerate()
        .map(|(i, n)| window_msg("s1", &"x".repeat(*n), &format!("2026-09-15T00:00:{i:02}Z")))
        .collect();
    out.push(window_msg("s1", user_message, "2026-09-15T00:01:00Z"));
    out
}

#[test]
fn mika2295_ceiling_elides_oldest_first() {
    // 4 × 400 bytes of history = 1600, ceiling of 250 tokens = 1000 bytes.
    // Two of the four must go, and they must be the two oldest.
    let mut history = window_with_history(&[400, 400, 400, 400], "the question");
    history[0].content = format!("oldest{}", "x".repeat(394));
    history[3].content = format!("newest{}", "x".repeat(394));

    let t = truncate_history_to_token_budget(&mut history, 250);

    assert_eq!(t.truncated_messages, 2);
    assert_eq!(t.truncated_bytes, 800);
    // marker + 2 surviving history + the user message
    assert_eq!(history.len(), 4);
    assert!(history.iter().any(|m| m.content.starts_with("newest")));
    assert!(
        !history.iter().any(|m| m.content.starts_with("oldest")),
        "the oldest message is the first to go, never the newest"
    );
}

#[test]
fn mika2295_the_turn_s_own_question_is_never_elided() {
    // Even at the tightest ceiling there is, the last message survives:
    // eliding it would answer a question that was erased.
    let mut history = window_with_history(&[5_000, 5_000], "what should I do about #2295?");

    let t = truncate_history_to_token_budget(&mut history, 0);

    assert_eq!(t.truncated_messages, 2);
    assert_eq!(
        history.last().unwrap().content,
        "what should I do about #2295?"
    );
}

#[test]
fn mika2295_zero_is_the_omission_sentinel_and_empties_the_history() {
    // `Some(0)` inherits `ContextSummaryConfig`'s meaning: omit, do not "cap
    // at zero tokens". What it omits is the history — see the test above for
    // the half of that sentence the plan left implicit.
    let mut history = window_with_history(&[100, 100, 100], "q");

    truncate_history_to_token_budget(&mut history, 0);

    assert_eq!(history.len(), 2, "marker + the turn's own message only");
    assert_eq!(history[0].content, HISTORY_ELISION_MARKER);
    assert_eq!(history[1].content, "q");
}

#[test]
fn mika2295_elision_inserts_a_marker_rather_than_shrinking_in_silence() {
    let mut history = window_with_history(&[4_000], "q");

    truncate_history_to_token_budget(&mut history, 10);

    assert_eq!(
        history[0].content, HISTORY_ELISION_MARKER,
        "a window that shrinks silently teaches the model it has seen everything"
    );
    // The marker must not look like another ticket to AC7's detector.
    assert_eq!(
        build_context_window_fields(
            &history,
            prompt::HistoryScope::Agent,
            &[],
            1,
            4_000,
            chrono::Utc::now()
        )
        .distinct_sessions,
        1,
        "the marker carries the turn's own session_id"
    );
}

#[test]
fn mika2295_a_window_already_under_budget_is_left_exactly_alone() {
    let before = window_with_history(&[10, 10], "q");
    let mut after = before.clone();

    let t = truncate_history_to_token_budget(&mut after, 1_000);

    assert_eq!(t, HistoryTruncation::default());
    assert_eq!(after.len(), before.len());
    assert!(
        after.iter().all(|m| m.content != HISTORY_ELISION_MARKER),
        "no marker when nothing was elided"
    );
}

#[test]
fn mika2295_a_window_with_no_history_is_a_no_op_at_any_ceiling() {
    // Both degenerate shapes: empty, and the turn's own message alone. A
    // first architect pass under session scope is exactly the second one, so
    // this is the nominal case, not an edge case.
    for len in [0usize, 1] {
        let mut history: Vec<crate::db::SessionMessage> = (0..len)
            .map(|_| window_msg("s1", &"x".repeat(9_999), "2026-09-15T00:00:00Z"))
            .collect();

        let t = truncate_history_to_token_budget(&mut history, 0);

        assert_eq!(t, HistoryTruncation::default(), "len = {len}");
        assert_eq!(history.len(), len, "len = {len}");
    }
}

#[test]
fn mika2295_budget_converts_through_the_one_shared_estimator() {
    // Not a second estimator beside `truncate_to_token_budget`'s: 4 bytes per
    // token, the same constant. 100 tokens = 400 bytes, so a 400-byte history
    // fits and a 401-byte one does not.
    let mut fits = window_with_history(&[400], "q");
    assert_eq!(
        truncate_history_to_token_budget(&mut fits, 100),
        HistoryTruncation::default()
    );

    let mut over = window_with_history(&[401], "q");
    assert_eq!(
        truncate_history_to_token_budget(&mut over, 100).truncated_messages,
        1
    );
}
