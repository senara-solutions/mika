//! Les gardes structurelles de source de la boucle d'agent.
//!
//! Les scans de cette tranche établissent que les deux sites 5e portent la
//! nature de leur assertion (mika#2879), que tout appel LLM de ce module est
//! enveloppé d'un délai (mika#2342), que toute émission `turn_usage` dit ce
//! que son tour a produit (mika#1910), que chaque sortie vers l'utilisateur
//! normalise sa typographie (mika#2247), que la fraîcheur de config et la
//! fenêtre d'historique n'ont qu'un lecteur décisionnel (mika#2473,
//! mika#2425, mika#2305), et que la classification production/test d'un
//! chemin reste exacte sur un module de test extrait (mika#2321).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant d'`agent_loop`
//! — les items privés d'`agent_loop` et les helpers promus dans `tests/mod.rs`
//! y seraient visibles via `use super::*`. Cette tranche est la seule à ne pas
//! en porter : un scan de source lit un fichier et assertionne sur son texte,
//! il n'appelle rien de la boucle. L'import serait inutilisé, et `cargo build`
//! le dit.

// ===========================================================================
// mika#2879 — les deux sites 5e portent la nature de l'assertion
// ===========================================================================

/// Le préfixe d'événement des deux `warn!` de la garde 5e.
///
/// Un préfixe et non deux noms exacts : il couvre
/// `guard.unactioned_frequency_promise` **et** son `_uncorrected`, et il
/// couvrirait un troisième site 5e le jour où il naîtrait — ce qui est la
/// bonne borne, puisque la question posée est « tout site 5e porte-t-il la
/// nature ? » et non « ces deux-là la portent-ils ? ».
const FREQUENCY_PROMISE_EVENT_PREFIX: &str = "event = \"guard.unactioned_frequency_promise";

/// Les sites d'émission 5e qui n'émettent PAS `assertion_nature`.
///
/// Découpé sur `warn!(` : chaque segment est le corps d'une émission, et
/// celui qui nomme un événement 5e doit nommer la nature. Les commentaires
/// sont retirés d'abord ([`crate::source_scan::strip_comment_lines`]) — la
/// prose *à propos* du champ n'est pas une émission (motif mika#2050) — et
/// la moitié de production est isolée, puisque ce module de test cite les
/// deux tokens.
fn frequency_promise_sites_missing_nature(src: &str) -> (usize, Vec<String>) {
    let production =
        crate::source_scan::strip_comment_lines(crate::source_scan::production_half(src));
    let mut sites = 0usize;
    let mut missing = Vec::new();

    for segment in production.split("warn!(").skip(1) {
        if !segment.contains(FREQUENCY_PROMISE_EVENT_PREFIX) {
            continue;
        }
        // Borner au corps de CETTE émission : le segment court jusqu'au
        // prochain `warn!(`, mais l'événement peut être suivi de beaucoup
        // de code. On s'arrête à la ligne de l'événement, qui est la
        // dernière du corps dans l'idiome de ce fichier.
        let body = match segment.find(FREQUENCY_PROMISE_EVENT_PREFIX) {
            Some(i) => &segment[..i],
            None => segment,
        };
        sites += 1;
        if !body.contains("assertion_nature") {
            let event = segment[segment.find(FREQUENCY_PROMISE_EVENT_PREFIX).unwrap_or(0)..]
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            missing.push(event);
        }
    }

    (sites, missing)
}

/// **AC-d — le discriminant a un consommateur de production, aux DEUX sites.**
///
/// Ce que cette garde attrape et ce qu'elle n'attrape pas, dit une fois pour
/// qu'un relecteur n'ait pas à le déduire. Elle atteste que le champ est
/// **émis** par les deux sites 5e ; elle n'atteste pas que la valeur émise
/// est la bonne — ça, c'est
/// `evidence::guards::tests::mika2879_chaque_alternative_porte_la_nature_attendue`,
/// sur les douze alternatives. Les deux ensemble couvrent D2 ; aucune des
/// deux seule ne le couvre.
///
/// **Pourquoi une garde structurelle et pas un test moteur.** Le plan
/// (R3) offrait un `tracing-test` sur la branche 5e. `tracing-test` n'est
/// pas une dépendance de ce crate : l'ajouter mettrait `Cargo.toml` et
/// `Cargo.lock` dans le diff, ce qu'AC-f refuse. Le refus n'est donc pas une
/// question de marge — c'est une impossibilité de périmètre, et l'en-tête de
/// doctrine du module de `guards.rs` la consigne. Le vol 3 traverse cette
/// branche avec de vrais tests moteur.
#[test]
fn mika2879_les_deux_sites_5e_emettent_la_nature() {
    let this_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("agent_loop")
        .join("mod.rs");
    let src = std::fs::read_to_string(&this_file)
        .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", this_file.display()));

    let (sites, missing) = frequency_promise_sites_missing_nature(&src);

    // Anti-vacuité : une garde qui ne trouve aucun site se lit exactement
    // comme un arbre propre. Deux, et pas « au moins un » — le site du
    // résidu est précisément celui qu'on oublie, et c'est sur lui que les
    // deux populations resteraient confondues (classe mika#2205).
    assert_eq!(
        sites, 2,
        "mika#2879 — deux sites d'émission 5e attendus (le tir et le \
             résidu), {sites} trouvé(s). Zéro signifie que le découpage a \
             cessé de les voir : la garde ne vérifie plus rien."
    );

    assert!(
        missing.is_empty(),
        "mika#2879 — un site 5e émet sans la nature de l'assertion :\n{}\n\n\
             RÉSOLUTION : ajouter `assertion_nature = promise.nature.as_str()`. \
             Sans lui les deux natures que mika#2878 a séparées restent une \
             seule population sous un seul nom d'événement, donc non \
             soustractibles (classe mika#2205).",
        missing.join("\n")
    );
}

/// Le contrôle de bonne foi de la garde ci-dessus (R1, recette v).
///
/// Sans lui, « la garde voit un site nu » est indistinguable de « la garde
/// ne voit jamais rien ».
#[test]
fn mika2879_la_garde_des_sites_5e_rougit_sur_un_champ_retire() {
    let complet = concat!(
        "    warn!(\n",
        "        step,\n",
        "        assertion_nature = promise.nature.as_str(),\n",
        "        event = \"guard.unactioned_frequency_promise\",\n",
        "    );\n",
        "    warn!(\n",
        "        step,\n",
        "        assertion_nature = promise.nature.as_str(),\n",
        "        event = \"guard.unactioned_frequency_promise_uncorrected\",\n",
        "    );\n",
    );
    assert_eq!(
        frequency_promise_sites_missing_nature(complet),
        (2, Vec::new()),
        "la forme livrée doit se lire comme deux sites complets"
    );

    let ampute = complet.replace(
            "        assertion_nature = promise.nature.as_str(),\n        event = \"guard.unactioned_frequency_promise_uncorrected\"",
            "        event = \"guard.unactioned_frequency_promise_uncorrected\"",
        );
    let (sites, missing) = frequency_promise_sites_missing_nature(&ampute);
    assert_eq!(sites, 2, "les deux sites restent reconnus comme tels");
    assert_eq!(
        missing.len(),
        1,
        "le site amputé doit être vu, et nommé par son événement : {missing:?}"
    );
    assert!(
        missing[0].contains("_uncorrected"),
        "la garde doit nommer LEQUEL des deux sites est nu : {missing:?}"
    );

    // Bonne foi dans l'autre sens : un `warn!` d'une autre garde ne rentre
    // pas dans la population, même s'il parle de fréquence.
    let (sites, _) = frequency_promise_sites_missing_nature(
        "    warn!(step, event = \"guard.unactioned_hosting_claim\");\n",
    );
    assert_eq!(
        sites, 0,
        "une autre garde n'a pas à porter la nature d'une assertion de fréquence"
    );
}

// ===========================================================================
// mika#2342 — every LLM call of this module carries a safety net
// ===========================================================================

/// How many lines above a call the wrapping `timeout(` may sit.
///
/// Three covers both real shapes — the continuation's two-line form and the
/// watchdog's `match` form — plus the single-line form (window includes the
/// call's own line). Deliberately *not* wider: a `timeout` five lines above
/// is more likely to belong to something else, and a gate that accepts an
/// unrelated wrapper is a gate that passes the regression it exists to
/// catch.
const WATCHDOG_GUARD_LOOKBACK_LINES: usize = 3;

/// The detector behind [`mika2342_every_llm_call_is_wrapped_in_a_timeout`],
/// split out so the guard can be exercised on a fabricated string rather
/// than by breaking the real source (verification contract item 7).
///
/// Lexical, and says so: it looks for the call token and then for a
/// `tokio::time::timeout(` within the preceding
/// [`WATCHDOG_GUARD_LOOKBACK_LINES`]. It cannot parse Rust, so a
/// sufficiently creative formatting could fool it in either direction. That
/// is acceptable for what it defends — not a subtle behaviour, but the
/// *disappearance* of a wrapper, which is what a hurried refactor does.
fn unwrapped_deadline_call_sites(src: &str) -> Vec<String> {
    // Both tokens in halves: this function's own body is inside the file
    // the guard scans, so writing either one whole would make the gate its
    // own first offender (the `policy.rs:no_bare_agent_timeout_constant_remains`
    // motif).
    let call_token = concat!("send_message_", "with_deadline");
    let wrap_token = concat!("tokio::time::", "timeout(");

    let lines: Vec<&str> = src.lines().collect();
    let mut offenders = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // Comments are how this ticket explains itself; scanning them would
        // make the explanation the violation.
        if trimmed.starts_with("//") {
            continue;
        }
        // The call token appears in a *definition* too (`fn
        // send_message_with_deadline`), which is not a call site.
        if !line.contains(call_token) || trimmed.starts_with("async fn") {
            continue;
        }
        let from = i.saturating_sub(WATCHDOG_GUARD_LOOKBACK_LINES);
        let wrapped = lines[from..=i].iter().any(|l| l.contains(wrap_token));
        if !wrapped {
            offenders.push(format!("{}: {}", i + 1, trimmed));
        }
    }

    offenders
}

/// Every `send_message_with_deadline` in this module must be the argument
/// of a `tokio::time::timeout` (mika#2342 D5).
///
/// # Why a source scan and not a behavioural test
///
/// Removing the net breaks **no assertion**. The loop keeps working, every
/// existing test stays green, and the only change is that the call goes
/// unbounded and silent again — which is the entire defect of mika#2342: a
/// 27-minute call with no `llm_call completed`, no `turn_usage` and no
/// error. A regression that makes nothing false, only something invisible,
/// is the class this house guards by scanning source
/// (`policy::no_bare_agent_timeout_constant_remains`,
/// `scripts/check-loop-select.sh`,
/// `grooming_marker::no_grooming_regex_outside_this_module`).
///
/// # Scope, and what a third site means
///
/// This file only. `investigate.rs` wraps its own call and a future caller
/// outside `agent_loop/` is not covered — widening the perimeter would be a
/// new decision, not a silent extension.
///
/// The inventory is closed at **two** sites: the continuation turn and the
/// main `run_loop` call. Per the plan's Fire-Disposition FD1 there is **no
/// allowlist** — an allowlist born empty is just a place to put the next
/// violation instead of wrapping it. A third site means either the
/// inventory was wrong or a call was added: **halt and surface**, do not
/// adjust the guard.
#[test]
fn mika2342_every_llm_call_is_wrapped_in_a_timeout() {
    // The scan reads the production half only: the test code below writes
    // both tokens on purpose. The boundary comes from
    // `mika_common::source_guard` (mika#2398) rather than from a local
    // `split_once`, so a module-level `#[cfg(test)]` helper added above the
    // test block can no longer hide the rest of the file from this guard.
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let production = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));

    let offenders = unwrapped_deadline_call_sites(&production);
    assert!(
        offenders.is_empty(),
        "mika#2342: {} LLM call site(s) in `agent_loop/mod.rs` are no longer wrapped in a \
             `tokio::time::timeout`.\n{}\n\n\
             WHY THIS MATTERS: the agent envelope is checked at the TOP of each loop iteration, \
             so it can never interrupt a call that does not return. Without this wrapper the \
             provider's own reqwest timeout is the SOLE cancellation mechanism, and when it \
             failed (mika#2342, n=2) the call ran 27+ minutes leaving no completion, no \
             turn_usage and no error — the task stayed `in_progress` for ever.\n\
             FIX: wrap the call as `run_loop` does, sized on \
             `llm.worst_case_failure_secs() + LLM_WATCHDOG_MARGIN_SECS` — never on the remaining \
             deadline, which would re-open mika#848.",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The guard's positive control: it must actually fire on a bare call.
///
/// Written against a fabricated snippet rather than by editing the real
/// source — a guard verified only by its own green is a guard verified by
/// nothing.
#[test]
fn mika2342_guard_fires_on_an_unwrapped_call() {
    let bare = "        let llm_result = llm\n\
                    \x20           .send_message_with_deadline(request, Some(deadline.into()))\n\
                    \x20           .await;\n";
    assert_eq!(
        unwrapped_deadline_call_sites(bare).len(),
        1,
        "the guard must flag an unwrapped call — it is the exact shape mika#2342 removed"
    );

    let wrapped = "        let llm_result = match tokio::time::timeout(\n\
                       \x20           watchdog,\n\
                       \x20           llm.send_message_with_deadline(request, Some(deadline.into())),\n\
                       \x20       )\n";
    assert!(
        unwrapped_deadline_call_sites(wrapped).is_empty(),
        "the guard must accept the wrapped shape, or it would forbid the fix"
    );

    let commented = "        // llm.send_message_with_deadline(request, None) is wrapped below\n";
    assert!(
        unwrapped_deadline_call_sites(commented).is_empty(),
        "prose naming the call must not be a violation, or the guard forbids documenting itself"
    );
}

// ===========================================================================
// mika#1910 — every `turn_usage` emission says what its turn produced
// ===========================================================================

/// The index of the closing paren matching the `(` at `open`.
///
/// Depth-counting, string-literal aware. It cannot parse Rust — a `(` inside
/// a char literal or a raw string would fool it — and that is acceptable for
/// what it bounds: the argument list of one named call, whose real shapes are
/// in this file and carry neither.
fn matching_paren(src: &str, open: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split an argument list on its **top-level** commas, dropping the empty
/// tail a trailing comma leaves behind.
fn top_level_args(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut cur = String::new();
    for c in list.chars() {
        if in_str {
            cur.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let tail = cur.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out.retain(|a| !a.is_empty());
    out
}

/// Every invocation of the `turn_usage` field builder found in `src`, as
/// `(1-based line, last argument)`.
///
/// The last argument **is** the `response_chars` position: that parameter is
/// declared last on `build_turn_usage_fields`, which is the one funnel all
/// three emission sites traverse (`save_continuation_llm_call` reaches it
/// through its own wrapper). So "did this site declare what it measured?"
/// reduces to reading one argument, which a scan can do exactly.
fn turn_usage_emitter_sites(src: &str) -> Vec<(usize, String)> {
    // In halves: this function's body lives inside the file the guard scans,
    // so writing the token whole would make the gate its own first offender
    // — the `unwrapped_deadline_call_sites` motif one block up, and the
    // mika#2201 class (a lint that reddens on its own prose).
    let emitter = concat!("build_turn_usage", "_fields");

    // Comments are how this ticket explains itself, and the constant's own
    // doc-comment contains the word `None`. Strip line comments first,
    // preserving the line structure so reported numbers stay those of `src`.
    let cleaned: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut sites = Vec::new();
    let mut from = 0usize;

    while let Some(rel) = cleaned[from..].find(emitter) {
        let start = from + rel;
        from = start + emitter.len();

        // A definition (`fn build_turn_usage_fields(`) is not a call site.
        if cleaned[..start].trim_end().ends_with("fn") {
            continue;
        }
        // Only whitespace may sit between the token and its `(`, or this is
        // a mention rather than an invocation.
        let rest = &cleaned[start + emitter.len()..];
        let Some(open_rel) = rest.find('(') else {
            continue;
        };
        if !rest[..open_rel].trim().is_empty() {
            continue;
        }

        let open = start + emitter.len() + open_rel;
        let Some(close) = matching_paren(&cleaned, open) else {
            continue;
        };
        let args = top_level_args(&cleaned[open + 1..close]);
        let Some(last) = args.last() else {
            continue;
        };
        let line = cleaned[..start].matches('\n').count() + 1;
        sites.push((line, last.clone()));
    }

    sites
}

/// The detector behind [`mika1910_every_unmeasured_site_declares_itself`],
/// split out so the guard can be exercised on a fabricated string rather
/// than by breaking the real source (mika#1910 verification contract §7).
///
/// # Why the criterion is the DECLARATION and not the error arm
///
/// The natural predicate — *"is this `None` inside an `Err` arm?"* — is not
/// one a grep can settle, and both approximations fail in **both**
/// directions: a lookback for `Err(` misses an error site written otherwise
/// (`match … { e @ LlmError::… =>`, a `?` bubbling up, a helper), and it
/// accepts any `None` that happens to sit under a neighbouring `Err`. So the
/// question is moved from the context to the declaration: *"did the author
/// write the token that says **I know this site measures nothing**?"* — which
/// a scan answers exactly, and which documents the site into the bargain.
fn undeclared_unmeasured_sites(src: &str) -> Vec<String> {
    turn_usage_emitter_sites(src)
        .into_iter()
        .filter(|(_, last)| last == "None")
        .map(|(line, _)| format!("{line}: passes a bare `None` for `response_chars`"))
        .collect()
}

/// Every `turn_usage` emission site must declare what it measured — either a
/// real count, or [`RESPONSE_CHARS_UNMEASURED`] (mika#1910 U1).
///
/// # Why a source scan and not a behavioural test
///
/// Removing the measure breaks **no assertion**. The loop keeps working,
/// every existing test stays green, and the only change is that the line goes
/// mute again — which is the entire defect of mika#1910: the continuation
/// turn recorded `response_text = NULL` unconditionally, successes included,
/// so the one row that carries the class could not distinguish "produced a
/// summary" from "produced nothing". A regression that makes nothing false,
/// only something invisible, is the class this house guards by scanning
/// source (`mika2342_every_llm_call_is_wrapped_in_a_timeout` one block up,
/// `policy::no_bare_agent_timeout_constant_remains`,
/// `grooming_marker::no_grooming_regex_outside_this_module`).
///
/// # Scope, and what a fourth site means
///
/// This file only, and the inventory is closed at **three** sites — the loop's
/// `Ok` arm, the loop's `Err` arm, and the continuation turn. Per the plan's
/// Fire-Disposition there is **no allowlist**: an allowlist born empty is
/// just a place to put the next violation instead of measuring it. A fourth
/// site is **halt and surface** — whether it has a response to measure is a
/// question this guard cannot settle for its author.
#[test]
fn mika1910_every_unmeasured_site_declares_itself() {
    // The scan reads the production half only: the test code below writes a
    // bare `None` on purpose. The boundary comes from
    // `mika_common::source_guard` (mika#2398) rather than a local
    // `split_once("#[cfg(test)]")`, whose premise stopped being true at
    // mika#2310 (an extracted test module carries no such literal).
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let production = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));

    let offenders = undeclared_unmeasured_sites(&production);
    assert!(
        offenders.is_empty(),
        "mika#1910: {} `turn_usage` emission site(s) in `agent_loop/mod.rs` pass a bare \
             `None` for `response_chars`.\n{}\n\n\
             WHY THIS MATTERS: `response_chars` is the ONLY surface on which the mika#1910 class \
             is countable. `null` there means \"not measured\"; `0` means \"measured, and the \
             model produced nothing\" — which IS the class. A bare `None` collapses the two, and \
             the offline analyzer (`scripts/measure-empty-turns`) then classes the turn \
             `undetermined` instead of `empty_response`: the measurement silently loses exactly \
             the population the ticket exists to count.\n\
             FIX: pass the count when the call returned (`Some(0)` on an empty response, never \
             `None`), or the named constant RESPONSE_CHARS_UNMEASURED when no call returned. \
             Do NOT pass `0` on an error arm — nothing was measured there, and `0` would be a \
             readable lie (the mika#2331 rule on `request_bytes`, one struct away).",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The inventory is closed at three emission sites (mika#1910 U1).
///
/// Without this, a guard grown too narrow — a renamed builder, a changed
/// argument order — would pass by looking at nothing, and a green scan would
/// be indistinguishable from a healthy one (the mika#2205 class).
#[test]
fn mika1910_the_inventory_of_emission_sites_is_closed() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let production = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));

    let sites = turn_usage_emitter_sites(&production);
    assert_eq!(
        sites.len(),
        3,
        "mika#1910: expected exactly 3 `turn_usage` emission sites (loop Ok arm, loop Err \
             arm, continuation turn), found {}: {:?}.\n\
             A FOURTH SITE IS HALT-AND-SURFACE, not an allowlist entry: whether it has a \
             response to measure is a question this guard cannot settle for its author.\n\
             ZERO SITES means the scan stopped seeing the builder at all — repair the scan \
             before trusting its sibling's green.",
        sites.len(),
        sites
    );
}

/// The guard's positive and negative controls, on fabricated snippets.
///
/// Exercising it by breaking the real source is refused: the guard would
/// become untestable without reddening the repository.
#[test]
fn mika1910_guard_fires_on_a_bare_none() {
    let emitter = concat!("build_turn_usage", "_fields");
    let declared = concat!("RESPONSE_CHARS_", "UNMEASURED");

    // Positive control — the named constant is a declaration, not a violation.
    let good = format!(
        "            let fields = {emitter}(\n\
             \x20               step,\n\
             \x20               usage,\n\
             \x20               &stop,\n\
             \x20               false,\n\
             \x20               \"error\",\n\
             \x20               latency,\n\
             \x20               request_bytes,\n\
             \x20               None,\n\
             \x20               {declared},\n\
             \x20           );\n"
    );
    assert!(
        undeclared_unmeasured_sites(&good).is_empty(),
        "the named constant must be accepted, or the guard forbids the fix itself"
    );

    // Negative control 1 — a bare `None` at the `response_chars` position.
    let bare = format!(
        "            let fields = {emitter}(\n\
             \x20               step,\n\
             \x20               usage,\n\
             \x20               &stop,\n\
             \x20               false,\n\
             \x20               \"error\",\n\
             \x20               latency,\n\
             \x20               request_bytes,\n\
             \x20               None,\n\
             \x20               None,\n\
             \x20           );\n"
    );
    assert_eq!(
        undeclared_unmeasured_sites(&bare).len(),
        1,
        "the guard must flag a bare `None` — it is the exact shape mika#1910 removed"
    );

    // Negative control 2 — the SAME bare `None`, under a line containing
    // `Err(`. It must STILL be flagged: this is the control that separates
    // "the guard reads the declaration" from "the guard reads the
    // neighbourhood". Without it, a lookback grown by accident would pass.
    let under_err = format!(
        "            Err(e) => {{\n\
             \x20           let fields = {emitter}(\n\
             \x20               step,\n\
             \x20               usage,\n\
             \x20               &stop,\n\
             \x20               false,\n\
             \x20               \"error\",\n\
             \x20               latency,\n\
             \x20               request_bytes,\n\
             \x20               None,\n\
             \x20               None,\n\
             \x20           );\n\
             \x20       }}\n"
    );
    assert_eq!(
        undeclared_unmeasured_sites(&under_err).len(),
        1,
        "a bare `None` under an `Err(` line must still be flagged — the criterion is the \
             DECLARATION, never the neighbourhood. A grep cannot decide whether a line sits in \
             an error arm; it can decide whether its author wrote the token."
    );

    // Good faith — prose naming the shape must not be a violation, or the
    // guard forbids documenting itself (mika#2201, mika#2050).
    let commented = format!("            // {emitter}(.., None) was the pre-fix shape\n");
    assert!(
        undeclared_unmeasured_sites(&commented).is_empty(),
        "prose naming the call must not be a violation, or the guard forbids documenting \
             itself"
    );
}

// ===========================================================================
// mika#2247 — every user-facing output site normalises its typography
// ===========================================================================

/// How many lines above the tag-strip the normaliser may sit.
///
/// Two covers the single-line form and the three-line composition all three
/// real sites use. Deliberately *not* wider, for the sibling guard's reason:
/// a normaliser five lines up is more likely to belong to something else,
/// and a gate that accepts an unrelated wrapper passes the regression it
/// exists to catch.
const TYPOGRAPHY_GUARD_LOOKBACK_LINES: usize = 2;

/// The detector behind
/// [`mika2247_every_output_site_normalises_its_typography`], split out so
/// the guard can be exercised on a fabricated string rather than by
/// breaking the real source.
///
/// Lexical, and says so: it finds the tag-strip call and then looks for the
/// normaliser within the preceding [`TYPOGRAPHY_GUARD_LOOKBACK_LINES`]. It
/// cannot parse Rust. That is acceptable for what it defends — not a subtle
/// behaviour, but a **new output site** added without the normalisation,
/// which is what a hurried feature does.
fn unnormalised_output_sites(label: &str, src: &str) -> Vec<String> {
    // Both tokens in halves: this function lives inside the crate the guard
    // scans, so writing either one whole would make the gate its own first
    // offender (the `mika2342_*` motif above).
    let strip_token = concat!("strip_internal_", "tags(");
    let normalise_token = concat!("normalize_typography_", "for_persona(");

    let lines: Vec<&str> = src.lines().collect();
    let mut offenders = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // Prose naming the call is how this ticket explains itself; scanning
        // it would make the explanation the violation.
        if trimmed.starts_with("//") {
            continue;
        }
        if !line.contains(strip_token) {
            continue;
        }
        let from = i.saturating_sub(TYPOGRAPHY_GUARD_LOOKBACK_LINES);
        if !lines[from..=i].iter().any(|l| l.contains(normalise_token)) {
            offenders.push(format!("{label}:{}: {trimmed}", i + 1));
        }
    }

    offenders
}

/// Every production site of this crate that produces user-facing text must
/// compose the tag strip with the mika#2247 normaliser.
///
/// # Why a source scan and not a behavioural test
///
/// A fourth output site added without the normalisation breaks **no
/// assertion**. Every existing test stays green; the only change is that one
/// path starts emitting em-dashes to a family tenant again. A regression
/// that makes nothing false, only something un-normalised on one path, is
/// the class this house guards by scanning source
/// (`mika2342_every_llm_call_is_wrapped_in_a_timeout` two blocks up,
/// `grooming_marker::no_grooming_regex_outside_this_module`).
///
/// # Scope, and what a fourth site means
///
/// The whole `mika-agent` crate, production half only. The inventory is
/// closed at **three** sites — the EndTurn extraction, the continuation
/// turn, and the `send_message` tool — and there is **no allowlist**: an
/// allowlist born empty is just a place to put the next violation instead
/// of normalising it. A fourth site means either the inventory was wrong or
/// an output path was added: **halt and surface**, do not adjust the guard.
///
/// `mika-common`'s own `serialize_response_text` is deliberately outside the
/// perimeter: it feeds `llm_calls.response_text`, an observability sink, not
/// a user channel.
#[test]
fn mika2247_every_output_site_normalises_its_typography() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    let mut sites = 0usize;
    scanner.for_each(|path, production| {
        let label = path
            .strip_prefix(scanner.src_root())
            .unwrap_or(path)
            .display()
            .to_string();
        sites += production
            .lines()
            .filter(|l| {
                !l.trim_start().starts_with("//") && l.contains(concat!("strip_internal_", "tags("))
            })
            .count();
        offenders.extend(unnormalised_output_sites(&label, production));
    });

    assert!(
        offenders.is_empty(),
        "mika#2247: {} output site(s) strip internal tags without normalising the \
             typography.\n{}\n\n\
             WHY THIS MATTERS: the family persona prescribes simple ASCII punctuation, and \
             glm-5.2 emits em-dashes by style. An un-normalised site re-opens AC1 on that path \
             alone, silently — no test goes red, the tenant simply reads « … connaître — tu me \
             parles » again.\n\
             FIX: compose as the three existing sites do —\n  \
             mika_common::text::normalize_typography_for_persona(\n    \
             mika_common::llm::strip_internal_tags(…),\n    <persona>,\n  )\n\
             and normalise BEFORE the text is captured, persisted or measured, so the database, \
             the guards and the delivered message never diverge (mika#2136).",
        offenders.len(),
        offenders.join("\n")
    );

    assert_eq!(
        sites, 3,
        "mika#2247: the inventory is closed at three output sites (EndTurn extraction, \
             continuation turn, `send_message`). Found {sites}. A fourth site is halt-and-surface, \
             not an allowlist entry — decide whether it produces user-facing text, normalise it if \
             it does, and update this count with its reason."
    );
}

/// The guard's positive control: it must actually fire on a bare call.
///
/// Written against a fabricated snippet rather than by editing the real
/// source — a guard verified only by its own green is a guard verified by
/// nothing.
#[test]
fn mika2247_typography_guard_fires_on_an_unnormalised_site() {
    let bare = "        let cleaned = mika_common::llm::strip_internal_tags(text);\n";
    assert_eq!(
        unnormalised_output_sites("fixture.rs", bare).len(),
        1,
        "the guard must flag a bare tag strip — it is the exact shape mika#2247 replaced"
    );

    let composed = "        let cleaned = mika_common::text::normalize_typography_for_persona(\n\
                        \x20           mika_common::llm::strip_internal_tags(text),\n\
                        \x20           ctx.tier.persona_profile(),\n\
                        \x20       );\n";
    assert!(
        unnormalised_output_sites("fixture.rs", composed).is_empty(),
        "the guard must accept the composed shape, or it would forbid the fix"
    );

    let commented = "        // strip_internal_tags(text) is normalised just below\n";
    assert!(
        unnormalised_output_sites("fixture.rs", commented).is_empty(),
        "prose naming the call must not be a violation, or the guard forbids documenting itself"
    );
}

/// Groups of `match` arms that **dispatch on** `HistoryScope` — i.e. read the
/// scope in order to decide or render something.
///
/// The predicate is positional, not lexical, and that is what makes it usable:
/// the type name must appear **in the pattern**, left of the `=>`. So
/// `deserialize_history_scope`'s arms — which merely *produce* a
/// `HistoryScope` on the right of their `=>` — fall out on their own, with no
/// name-based allowlist that would have to be widened by hand every time a
/// legitimate site appears. Equality assertions (`assert_eq!(…, Scope::Agent)`)
/// carry no `=>` and fall out too.
///
/// Arms within two lines of each other are one site: a `match` has several
/// arms and counting lines would answer a different question from the one
/// asked ("how many readers?").
fn scope_match_sites(src: &str) -> Vec<Vec<(usize, String)>> {
    let hits: Vec<(usize, String)> = src
        .lines()
        .enumerate()
        .filter(|(_, line)| match line.find("=>") {
            Some(arrow) => line[..arrow].contains("HistoryScope::"),
            None => false,
        })
        .map(|(n, line)| (n + 1, line.trim().to_string()))
        .collect();

    let mut sites: Vec<Vec<(usize, String)>> = Vec::new();
    for hit in hits {
        match sites.last_mut() {
            Some(last) if hit.0.saturating_sub(last[last.len() - 1].0) <= 2 => last.push(hit),
            _ => sites.push(vec![hit]),
        }
    }
    sites
}

/// `HistoryScope` reader sites in one file, production half only.
///
/// Two exclusions compose, in this order: a file whose **path** is test
/// code has no production half at all (mika#2321, [`crate::source_scan`]);
/// in any other file the test regions are masked by
/// [`mika_common::source_guard`] (mika#2398) — never by truncating at the
/// first `#[cfg(test)]`, which cut `prompt.rs` at line 142 on a doc comment
/// that merely mentions the marker. Masking keeps the file's own line
/// numbers, so a site found here is named where it lives.
fn scope_reader_sites(path: &std::path::Path, src: &str) -> Vec<Vec<(usize, String)>> {
    if crate::source_scan::is_test_source_path(path) {
        return Vec::new();
    }
    scope_match_sites(&mika_common::source_guard::mask_test_regions(src))
}

/// Tokens that make a line an *addition* rather than a read.
///
/// `+=` and the three checked forms are unambiguous; bare `+` is matched
/// with its surrounding spaces so `run_usage` inside a `format!("{a}+{b}")`
/// or a doc link is not swept in.
const MIKA1883_ADDITION_TOKENS: &[&str] =
    &[" + ", "+=", "saturating_add", "checked_add", "wrapping_add"];

/// The four `LlmUsage` fields an open-coded fold would touch.
const MIKA1883_USAGE_FIELDS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];

/// Lines under `agent_loop/` that add usage without going through the
/// helper, as `(1-based line, trimmed text)`.
fn mika1883_open_coded_fold_sites(src: &str) -> Vec<(usize, String)> {
    src.lines()
        .enumerate()
        .filter(|(_, line)| {
            let mentions_usage = line.contains("run_usage")
                || MIKA1883_USAGE_FIELDS.iter().any(|f| line.contains(f));
            let adds = MIKA1883_ADDITION_TOKENS.iter().any(|t| line.contains(t));
            let via_helper = line.contains("LlmUsage::accumulate");
            mentions_usage && adds && !via_helper
        })
        .map(|(i, line)| (i + 1, line.trim().to_string()))
        .collect()
}

/// **AC9** — the turn total is folded at one place, and only there.
///
/// The writer counterpart of
/// `mika1883_both_client_surfaces_read_the_one_reader`, and the only one of
/// the two that can see a **second fold**. `LlmUsage::accumulate` has its own
/// unit test, which attests that `None + Some(n) = Some(n)` is right — it
/// attests nothing about whether the two addition sites *use* it. A second
/// site that re-wrote the `Option` merge by hand would serve a wrong total
/// with every existing test green, which is the silent class this repo closes
/// with a scan wherever it has met it (`mika2131_*`, `mika2220_*`).
///
/// Shipped with **no allowlist**. That is not optimism: `run_usage` is born
/// in this change and its two addition sites are created by it, so the first
/// entry anyone would want to add here is exactly the divergence the scan
/// exists to refuse. Resolution when it fires: **remove the second site**,
/// never allowlist it — the rule `ACTOR_READING_PREDICATES_ALLOWED`
/// (mika#2323) states for its own list shipped empty.
#[test]
fn mika1883_run_usage_accumulates_only_via_the_one_helper() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let agent_loop_root = src_root.join("agent_loop");
    let mut offenders: Vec<String> = Vec::new();

    scanner.for_each_under(&agent_loop_root, |path, production| {
        if crate::source_scan::is_test_source_path(path) {
            return;
        }
        let rel = path.strip_prefix(&src_root).unwrap_or(path).display();
        for (line, text) in mika1883_open_coded_fold_sites(production) {
            offenders.push(format!("{rel}:{line}: {text}"));
        }
    });

    assert!(
        offenders.is_empty(),
        "mika#1883 — a second site folds usage by hand instead of calling \
             `LlmUsage::accumulate`. Remove it; do not allowlist it — two folds \
             that can disagree produce a wrong total with every test green:\n{}",
        offenders.join("\n")
    );
}

/// Good-faith control for the scan above: it really does catch the shape it
/// claims to, rather than being a predicate that matches nothing.
#[test]
fn mika1883_the_fold_scan_catches_an_open_coded_site() {
    let offending = "fn f() {\n    run_usage.input_tokens += response.usage.input_tokens;\n}\n";
    assert_eq!(
        mika1883_open_coded_fold_sites(offending).len(),
        1,
        "the scan must catch a hand-written fold"
    );

    let legal = "fn f() {\n    run_usage = LlmUsage::accumulate(run_usage, Some(&u));\n}\n";
    assert!(
        mika1883_open_coded_fold_sites(legal).is_empty(),
        "a call to the helper is the legal shape and must not be flagged"
    );

    let a_mere_read = "fn f() {\n    let n = response.usage.input_tokens;\n}\n";
    assert!(
        mika1883_open_coded_fold_sites(a_mere_read).is_empty(),
        "reading a usage field is not folding it — flagging reads would make \
             the guard fire on `build_turn_usage_fields` and get disarmed"
    );
}

/// One `detect_config_change(` call site, with everything the AC7 assertion
/// needs to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mika2473Site {
    /// Path relative to the crate's `src/`, e.g. `agent_loop/mod.rs`.
    file: String,
    /// 1-based line inside that file.
    line: usize,
    /// The `fn` the call sits in.
    enclosing: String,
    /// The trimmed source line, so a failure reads without opening the file.
    text: String,
}

impl std::fmt::Display for Mika2473Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            file,
            line,
            enclosing,
            text,
        } = self;
        write!(f, "{file}:{line} (in `{enclosing}`): {text}")
    }
}

/// Every `detect_config_change(` call site in one Rust source, tagged with
/// the file it was found in.
///
/// **Prose is not a call site.** A line whose content starts with `//`
/// *names* the primitive without calling it; flagging those would make the
/// only way to keep the guard green to stop naming the contract in comments,
/// which is how a structural guard gets disarmed by the people it serves.
/// `server/mod.rs` names it in exactly that way, one line above
/// `note_config_at_boot`.
fn mika2473_freshness_check_sites(file: &str, src: &str) -> Vec<Mika2473Site> {
    let mut enclosing = String::from("<no enclosing fn>");
    let mut sites = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let text = line.trim();
        if let Some(name) = mika2473_declared_fn_name(text) {
            enclosing = name;
        }
        if text.starts_with("//") {
            continue;
        }
        if text.contains("detect_config_change(") {
            sites.push(Mika2473Site {
                file: file.to_string(),
                line: i + 1,
                enclosing: enclosing.clone(),
                text: text.to_string(),
            });
        }
    }
    sites
}

/// `fn f(` / `async fn f(` / `pub(crate) async fn f(` → `f`; anything else
/// that merely mentions `fn ` (a doc comment, a `let` binding) → `None`.
fn mika2473_declared_fn_name(trimmed: &str) -> Option<String> {
    let (head, rest) = trimmed.split_once("fn ")?;
    let is_declaration = head.split_whitespace().all(|token| {
        matches!(
            token,
            "pub" | "pub(crate)" | "pub(super)" | "async" | "const" | "unsafe" | "extern"
        )
    });
    if !is_declaration {
        return None;
    }
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// **AC7 (mika#2473)** — the config-freshness check has exactly one call
/// site **in the whole crate**, and that site is the funnel.
///
/// `load_agent_context` is the single entry point of the three loops —
/// conversation (`run_agent`), silent, and team (`run_team_agent_inner_impl`)
/// — so one call there evaluates every turn that runs on this process
/// (KTD5). A second call, added by an author who did not know the funnel
/// existed, costs a second `stat` per turn and — the real damage — splits
/// the "one report per distinct mtime" contract (R9/KTD4) across two callers
/// of a dedup map keyed by `agent_id`, not by call site: whichever site ran
/// first would silence the other, and which one that is would depend on the
/// loop.
///
/// # Why the whole crate and not `agent_loop/mod.rs`
///
/// Because the damage does not care which file the second site lives in, and
/// a scan of one file is *most* blind exactly where a stranger to this
/// contract would write: `server/`, `teams/`, `task_engine/`. A guard that
/// green-lights the case it exists to refuse is worse than none — it reads
/// as coverage. The one-file form shipped that way and its companion control
/// only ever injected a second site into the *same* file, which is the case
/// the scan did cover.
///
/// Shipped with **no allowlist**, and the population at HEAD is **one**. So
/// the first entry anyone would want to add here is exactly the second site
/// this scan exists to refuse. Conduct when it fires: **remove the second
/// call site**, never allowlist it. The same rule
/// `mika1883_run_usage_accumulates_only_via_the_one_helper` states for its
/// own empty list, one module over.
#[test]
fn mika2473_the_freshness_check_sits_in_the_one_funnel() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let mut sites: Vec<Mika2473Site> = Vec::new();
    scanner.for_each(|path, production| {
        let label = path
            .strip_prefix(&src_root)
            .unwrap_or(path)
            .display()
            .to_string();
        sites.extend(mika2473_freshness_check_sites(&label, production));
    });

    let rendered = sites
        .iter()
        .map(Mika2473Site::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        sites.len(),
        1,
        "mika#2473 AC7 — `detect_config_change` must be called exactly once \
             in this crate. Remove the extra call site; do not allowlist it — two \
             sites share one dedup key and silence each other:\n{rendered}"
    );
    assert_eq!(
        sites[0].enclosing, "load_agent_context",
        "mika#2473 KTD5 — the one call must sit in `load_agent_context`, the \
             funnel of the three loops. A guard in a single loop is blind to the \
             other two:\n{rendered}"
    );
    assert_eq!(
        sites[0].file,
        std::path::Path::new("agent_loop")
            .join("mod.rs")
            .display()
            .to_string(),
        "mika#2473 KTD5 — and `load_agent_context` lives in `agent_loop/mod.rs`. \
             A same-named fn elsewhere is not the funnel:\n{rendered}"
    );
}

/// Good-faith control for the scan above: it catches the shape it claims to,
/// rather than being a predicate that matches nothing.
///
/// The second site is injected into a **different file**, which is the case
/// the one-file form of this guard could not see: it is the whole reason the
/// scan became a crate walk.
#[test]
fn mika2473_the_funnel_scan_catches_a_second_site() {
    let funnel = "async fn load_agent_context(db: &AsyncDatabase) -> Result<AgentContext> {\n    \
             if let Some(finding) = mika_common::llm::detect_config_change(db.agent_id()) {}\n}\n";
    let one = mika2473_freshness_check_sites("agent_loop/mod.rs", funnel);
    assert_eq!(one.len(), 1, "the legal shape is one site");
    assert_eq!(
        one[0].enclosing, "load_agent_context",
        "and it is attributed to the funnel"
    );
    assert_eq!(one[0].file, "agent_loop/mod.rs", "and to its file");

    let same_file = format!(
        "{funnel}\nasync fn run_team_agent_inner_impl() {{\n    \
             let _ = mika_common::llm::detect_config_change(\"mika-arch\");\n}}\n"
    );
    let both = mika2473_freshness_check_sites("agent_loop/mod.rs", &same_file);
    assert_eq!(
        both.len(),
        2,
        "a second call site in the same file is caught"
    );
    assert_eq!(
        both[1].enclosing, "run_team_agent_inner_impl",
        "and named by the fn that added it"
    );

    // The case the one-file scan was blind to: a stranger to this contract
    // writes the second call where the turn passes through *their* module.
    let elsewhere = "pub async fn init_agent(agent_name: &str) {\n    \
             let _ = mika_common::llm::detect_config_change(agent_name);\n}\n";
    let mut across = one.clone();
    across.extend(mika2473_freshness_check_sites("server/mod.rs", elsewhere));
    assert_eq!(
        across.len(),
        2,
        "a second call site in ANOTHER file must be caught — this is the \
             population the crate walk exists for"
    );
    assert_eq!(across[1].file, "server/mod.rs", "and named by its file");
    assert_eq!(across[1].enclosing, "init_agent");

    let prose = "/// The funnel calls `detect_config_change(` once.\nfn f() {}\n";
    assert!(
        mika2473_freshness_check_sites("server/mod.rs", prose).is_empty(),
        "naming the primitive in a comment is not calling it — flagging \
             prose is how this guard would get disarmed"
    );
}

/// **T5** — the scope has one decisional reader, and it is
/// `run_agent`'s `scoped_session_id`.
///
/// A second reader would make no decision *wrong* the day it is written; it
/// would make two answers to one question able to drift apart — the class
/// `grooming_marker` had to engrave once (mika#2158: a copied regex whose own
/// comment said "Mirrors …" and then missed two widenings).
///
/// Exactly **three** sites are expected, all under `agent_loop/`: the
/// cascade (`context_history::resolve`, mika#2425), the decision
/// (`scoped_session_id`) and the rendering (`history_scope_label`). A fourth
/// is halt-and-surface, not an allowlist entry — whether it is a legitimate
/// rendering or a second decision is a question this guard cannot answer for
/// you.
///
/// **mika#2425 amended this guard rather than excepting itself from it**, and
/// the difference matters. What the guard protects is written in its own
/// message: *two answers to "which rows may this window draw from?" can drift
/// apart without breaking anything visible.* `context_history::resolve` is
/// not a second answer — it **produces** the single answer `scoped_session_id`
/// consumes, and `mika2425_identity_context_history_has_a_single_reader`
/// guarantees that consumer reads nothing else. Two grep-visible gestures:
/// the count goes `2 → 3` and names the third site, and the path constraint
/// relaxes from `agent_loop/mod.rs` to `agent_loop/` — the invariant as
/// written is *the readers live in the loop*, and the resolver is in the loop.
/// The constraint is not relaxed beyond that directory.
///
/// **The production half is read by [`mika_common::source_guard`]
/// (mika#2398).** Truncating at the first `#[cfg(test)]`, as this guard did,
/// cut `prompt.rs` at line 142 — a doc comment that merely *mentions* the
/// marker — and left 1 886 of its lines unread. The scan was not wrong about
/// what it saw; it had stopped seeing.
#[test]
fn mika2305_the_scope_has_a_single_decisional_reader() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let mut sites: Vec<String> = Vec::new();

    scanner.for_each(|path, production| {
        let rel = path.strip_prefix(&src_root).unwrap_or(path).display();
        for site in scope_reader_sites(path, production) {
            sites.push(format!("{rel}:{}: {}", site[0].0, site[0].1));
        }
    });

    assert_eq!(
        sites.len(),
        3,
        "mika#2305 (count amended by mika#2425) — expected exactly three readers of \
             `HistoryScope` outside deserialization: the cascade in \
             `context_history::resolve`, the decision in `run_agent` \
             (`scoped_session_id`) and the rendering in `history_scope_label`. \
             Found {}:\n{}\n\nIf you added a second *decision*, route it through \
             `scoped_session_id` instead — two answers to \"which rows may this window \
             draw from?\" can drift apart without breaking anything visible. If a \
             rustfmt reflow split the resolver's `match` into two sites, put its arms \
             back on one line each rather than raising this number.",
        sites.len(),
        sites.join("\n")
    );
    assert!(
        sites
            .iter()
            .all(|s| s.starts_with("agent_loop/") || s.starts_with("agent_loop\\")),
        "mika#2305 — a reader of `HistoryScope` lives outside `agent_loop/`:\n{}",
        sites.join("\n")
    );
}

/// **T5b — good-faith control for T5.** The predicate must redden on a
/// decisional `match` added elsewhere, and must stay silent on the three
/// shapes that legitimately name the type.
///
/// Without this, T5 could be green because it looks at nothing — which is the
/// exact failure mode T5 exists to make visible on the other axis.
#[test]
fn mika2305_the_scan_reddens_on_a_decisional_match_added_elsewhere() {
    let offending = r#"
            fn somewhere_else(scope: HistoryScope) -> usize {
                match scope {
                    HistoryScope::Session => 1,
                    HistoryScope::Agent => 20,
                }
            }
        "#;
    assert_eq!(
        scope_match_sites(offending).len(),
        1,
        "the scan must see a decisional match added outside the production site"
    );

    // Deserialization: the type is *produced* on the right of the arrow.
    let deserialization = r#"
            Some(toml::Value::String(s)) if s.eq_ignore_ascii_case("agent") => Ok(HistoryScope::Agent),
            Some(toml::Value::String(s)) if s.eq_ignore_ascii_case("session") => Ok(HistoryScope::Session),
        "#;
    assert!(
        scope_match_sites(deserialization).is_empty(),
        "deserialization produces the type, it does not read it — it must not count"
    );

    // Equality assertions carry no arrow.
    let assertion = "assert_eq!(identity.context.history.scope, HistoryScope::Agent);";
    assert!(
        scope_match_sites(assertion).is_empty(),
        "an equality assertion is not a reader"
    );

    // A `#[cfg(test)]` tail is not production.
    let with_test_tail = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    match s { HistoryScope::Agent => 1, HistoryScope::Session => 2 };\n}\n";
    assert!(
        scope_match_sites(&mika_common::source_guard::mask_test_regions(
            with_test_tail
        ))
        .is_empty(),
        "the test tail must be masked before scanning"
    );

    // mika#2398 — and the shape the previous `split at the first marker`
    // rule got wrong: a module-level test helper ABOVE the production this
    // guard must read. Truncating there is what left `prompt.rs` unread
    // past line 142.
    let helper_first = "\
#[cfg(test)]
fn helper() -> u32 {
    1
}

fn prod(scope: HistoryScope) -> usize {
    match scope {
        HistoryScope::Session => 1,
        HistoryScope::Agent => 20,
    }
}
";
    assert_eq!(
        scope_match_sites(&mika_common::source_guard::mask_test_regions(helper_first)).len(),
        1,
        "a decisional match AFTER a module-level test helper must still be seen — \
             the truncating rule this guard used to apply would have missed it"
    );
}

// -- mika#2425: the declared floor has ONE reader -----------------------

/// Production sites exempted from [`mika2425_identity_context_history_has_a_single_reader`].
///
/// **Shipped empty, and the resolution when the scan fires is to REMOVE the
/// second reader, never to add an entry here.** Same contract as
/// `ACTOR_READING_PREDICATES_ALLOWED` (mika#2323) and the empty allowlist of
/// `mika1883_run_usage_accumulates_only_via_the_one_helper` — a list born
/// empty is a slot for the next lapse, so its emptiness is itself asserted.
///
/// If an exception is ever genuinely warranted, the entry must name its
/// follow-up ticket (`mika#NNNN`), which the self-cleaning assertion below
/// enforces.
const CONTEXT_HISTORY_READERS_ALLOWED: &[&str] = &[];

/// Production lines reading the declared `[context.history]` block off an
/// identity, as `(1-based line, trimmed text)`.
///
/// The needle is the **dotted field path** (`.context.history`), not the
/// TOML section header `[context.history]`: the second names the block in a
/// doc comment, a template literal or a `CODE_OWNED_IDENTITY_SECTIONS`
/// entry without reading it, and sweeping those in would make the guard
/// unusable on the very files that legitimately describe the section.
///
/// A **comment line is not a reader**, and that exclusion is load-bearing
/// rather than cosmetic. This guard's own decision site carries three
/// paragraphs naming the field to say why it must be read exactly once, and
/// an unanchored predicate accused them — the same shape as the mika#2050
/// Signal S false positive, where a grep matched prose a pilot had written
/// *about* the signal. Anchoring on comments is what keeps the guard
/// readable enough to be trusted; an in-line trailing comment does not
/// exempt the code preceding it, since the line no longer *starts* with the
/// marker.
///
/// Two further exclusions compose, in this order and for the reasons
/// [`scope_reader_sites`] states: a file whose **path** is test code has no
/// production half (mika#2321); anywhere else the test regions are masked
/// (mika#2398), which keeps the file's own line numbers.
fn identity_history_reader_sites(path: &std::path::Path, src: &str) -> Vec<(usize, String)> {
    if crate::source_scan::is_test_source_path(path) {
        return Vec::new();
    }
    mika_common::source_guard::mask_test_regions(src)
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim().to_string()))
        .filter(|(_, line)| {
            line.contains(".context.history") && !line.starts_with("//") && !line.starts_with('*')
        })
        .collect()
}

/// **D3** — the identity's declared window has exactly one production reader.
///
/// The whole mika#2425 cascade rests on that number being one: the declared
/// value is a **floor** that the `customer_config` half may only narrow, and
/// a second site reading `identity.context.history` directly would apply the
/// floor without the narrowing — silently, with every behavioural assertion
/// still green, because both answers are individually plausible. That is the
/// class `grooming_marker` had to close once (mika#2158), where promotion
/// and dispatch routing answered the same question differently for months.
#[test]
fn mika2425_identity_context_history_has_a_single_reader() {
    assert!(
        CONTEXT_HISTORY_READERS_ALLOWED
            .iter()
            .all(|entry| entry.contains("mika#")),
        "mika#2425 — every entry of CONTEXT_HISTORY_READERS_ALLOWED must name the \
             follow-up ticket that will remove it. An exemption nobody owns is how a \
             guard stops guarding. Current list: {CONTEXT_HISTORY_READERS_ALLOWED:?}"
    );

    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let mut sites: Vec<String> = Vec::new();

    scanner.for_each(|path, production| {
        let rel = path
            .strip_prefix(&src_root)
            .unwrap_or(path)
            .display()
            .to_string();
        if CONTEXT_HISTORY_READERS_ALLOWED
            .iter()
            .any(|allowed| rel.contains(allowed))
        {
            return;
        }
        for (line, text) in identity_history_reader_sites(path, production) {
            sites.push(format!("{rel}:{line}: {text}"));
        }
    });

    assert_eq!(
        sites.len(),
        1,
        "mika#2425 — expected exactly ONE production reader of the identity's \
             `[context.history]`: the site that feeds `context_history::resolve`. \
             Found {}:\n{}\n\nIf you added a second one, route it through the resolver \
             instead of allowlisting it — a site reading the declared value directly \
             applies the role's floor WITHOUT the per-tenant narrowing, and nothing \
             visible breaks.",
        sites.len(),
        sites.join("\n")
    );
    assert!(
        sites[0].starts_with("agent_loop/mod.rs:") || sites[0].starts_with("agent_loop\\mod.rs:"),
        "mika#2425 — the one reader must be the decision site in `agent_loop/mod.rs`, \
             where the resolved scope is consumed:\n{}",
        sites[0]
    );
}

/// **D3b — good-faith control for D3.**
///
/// D3 could be green because it looks at nothing, which is the exact failure
/// mode D3 exists to make visible. So the predicate is shown to redden on a
/// reader added elsewhere, and to stay silent on the four shapes that name
/// the block without reading it.
#[test]
fn mika2425_the_reader_scan_reddens_on_a_second_reader() {
    let prod = std::path::Path::new("src/somewhere.rs");

    let offending = "fn elsewhere(identity: &Identity) -> HistoryScope {\n    \
             identity.context.history.scope\n}\n";
    assert_eq!(
        identity_history_reader_sites(prod, offending).len(),
        1,
        "the scan must see a reader added outside the decision site"
    );

    // 1. The TOML section header in a doc comment or a template literal.
    let header = "/// `[context.history]` bounds the window.\n\
             const IDENTITY: &str = \"[context.history]\\nscope = \\\"session\\\"\\n\";\n";
    assert!(
        identity_history_reader_sites(prod, header).is_empty(),
        "naming the TOML section is not reading the field"
    );

    // 2. The section path as data (CODE_OWNED_IDENTITY_SECTIONS).
    let as_data = "const CODE_OWNED: &[&str] = &[\"context.history\", \"context.summary\"];\n";
    assert!(
        identity_history_reader_sites(prod, as_data).is_empty(),
        "the section path as a string datum is not a field read"
    );

    // 3. A read inside a masked test region.
    let in_tests = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    \
             assert_eq!(identity.context.history.scope, HistoryScope::Agent);\n}\n";
    assert!(
        identity_history_reader_sites(prod, in_tests).is_empty(),
        "a test region is masked before scanning"
    );

    // 4. A file whose path is test code has no production half at all.
    let test_path = std::path::Path::new("src/db/tests/harnais.rs");
    assert!(
        identity_history_reader_sites(test_path, offending).is_empty(),
        "mika#2321 — a file under `tests/` is test code whatever its contents"
    );

    // 5. Prose ABOUT the field — the shape that accused this guard's own
    // decision site, and the mika#2050 Signal S class. A comment cannot read.
    let prose = "\
// This is the single production reader of `identity.context.history`.
/// Reading `ctx.identity.context.history.max_tokens` here would drop the narrowing.
 * `identity.context.history` is the role floor.
";
    assert!(
        identity_history_reader_sites(prod, prose).is_empty(),
        "prose naming the field is not a read — an unanchored predicate accuses \
             the very comments that explain why the field has one reader"
    );

    // …but a trailing comment does not exempt the code before it.
    let trailing = "    let h = &ctx.identity.context.history; // the role floor\n";
    assert_eq!(
        identity_history_reader_sites(prod, trailing).len(),
        1,
        "only a line that STARTS with a comment marker is prose"
    );
}

/// **T5c — good-faith control for the mika#2321 path classification.**
///
/// Sibling of T5b on the other axis: T5b proves the *predicate* still sees a
/// decisional match, this one proves the *file classification* has not been
/// widened into uselessness. A classification that exempted too much would
/// leave T5 green while it scanned nothing — the same vacuous-guard failure,
/// reached from the other side.
///
/// The file it exists for is the extracted test module: one carries no
/// `#[cfg(test)]` literal, so `production_half` alone returns it whole and
/// every legitimate mention in a fixture counts as a production reader.
#[test]
fn mika2321_scope_reader_sites_keeps_production_and_drops_test_paths() {
    let offending = r#"
            fn somewhere_else(scope: HistoryScope) -> usize {
                match scope {
                    HistoryScope::Session => 1,
                    HistoryScope::Agent => 20,
                }
            }
        "#;

    assert_eq!(
        scope_reader_sites(
            std::path::Path::new("/repo/crates/mika-agent/src/server/new_path.rs"),
            offending,
        )
        .len(),
        1,
        "the guard no longer sees a decisional match in a production file — \
             it has gone vacuous"
    );

    for test_path in [
        // `/tests/` segment — the shape mika#2321 creates in bulk.
        "/repo/crates/mika-agent/src/db/tests/tasks.rs",
        // bare `tests.rs` — the shape that was already in the hole.
        "/repo/crates/mika-agent/src/perimeter/tests.rs",
    ] {
        assert!(
            scope_reader_sites(std::path::Path::new(test_path), offending).is_empty(),
            "{test_path} is still scanned as production"
        );
    }
}
