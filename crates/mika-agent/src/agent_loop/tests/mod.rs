//! Tests de la boucle d'agent — le `mod tests` qui vivait inline dans
//! `agent_loop/mod.rs`, extrait tel quel (mika#3023).
//!
//! # Pourquoi un répertoire, et pas un `agent_loop/tests.rs`
//!
//! `perimeter::rules` classe **Mechanical** tout chemin portant un segment
//! `/tests/`, mais **DecisionCore** un `some_module/tests.rs` isolé — par choix
//! fail-closed explicite et commenté (`perimeter/tests.rs`, « the file itself
//! lives next to production code and might be accidentally cfg-flipped »). Or
//! une PR DECISION-CORE n'est pas dé-draftée automatiquement (mika#2286) : un
//! fichier `agent_loop/tests.rs` ferait donc de **chaque PR future touchant un
//! test de la boucle** un draft en attente d'un geste humain, c'est-à-dire
//! exactement le coût que mika#3023 existe pour retirer, déplacé d'un check
//! rouge vers un draft. Le répertoire l'évite par construction.
//!
//! **Ne « simplifiez » donc pas ce répertoire en un fichier plat.**
//!
//! # Pourquoi un seul fichier aujourd'hui
//!
//! Le précédent mika#2321 a réparti `db` par thème parce que sa région
//! append-only grossissait contre le plafond. Ici les deux grandeurs ne sont
//! pas du même ordre : `agent_loop/mod.rs` n'avait plus que 1 641 octets de
//! marge sous `LARGE_FILE_LIMIT`, ce fichier en a ~644 000 à sa naissance —
//! plus de 200 PR au débit mesuré. Le découpage thématique, lui, est un
//! arbitrage de 418 `#[test]` et de leurs helpers partagés, et le bloc n'offre
//! que 32 marqueurs de section de premier niveau. Il est **renvoyé à un ticket
//! de suivi**, pas oublié.
//!
//! Le report est gratuit plutôt que reporté-avec-dette, pour deux raisons :
//! `source_guard::test_only_modules` a une passe 2 à point fixe — dans un
//! fichier déjà classé test-only, toute déclaration de module l'est aussi, et
//! « so is every file under a test-only module's directory » — donc un
//! découpage ultérieur ne demande **aucune** retouche de garde ; et aucun nom
//! de test ne change ici, puisqu'un `mod tests` inline et un `mod tests;` sur
//! fichier donnent le même chemin de module `agent_loop::tests::<nom>`.
//!
//! # Ce que la déclaration `mod tests;` sans visibilité achète
//!
//! Le parent déclare `#[cfg(test)] mod tests;` **sans** `pub(crate)`, et ce
//! n'est pas un détail :
//!
//! - **Personne n'importe `agent_loop::tests`.** Les occurrences de ce chemin
//!   sous `crates/` sont toutes des mentions en prose. C'est la différence avec
//!   `db`, dont `tests/mod.rs` doit documenter que `skills::executor` importe
//!   `crate::db::tests::*`. Ici il n'y a aucun contrat inter-modules à
//!   préserver, donc `pub(crate)` serait une extension de visibilité sans
//!   lecteur.
//! - **Cette orthographe garde l'ancre de `source_scan::production_half`
//!   satisfaite.** L'ancre est le littéral `"\n#[cfg(test)]\nmod tests"` ;
//!   `pub(crate) mod tests;` ne la matche pas. La garder vivante laisse la
//!   coupure production/test de `mod.rs` se comporter après le déplacement
//!   comme elle se comportait avant — une invariance de moins à vérifier.

use super::*;
use crate::skills::index::{ResolvedSkillTool, SkillEntry};
use crate::skills::manifest::{SkillInfo, SkillManifest, ToolHandler, Triggers};
use crate::test_utils::test_helpers::test_async_db;
use mika_common::claude::ToolDefinition;
use std::path::PathBuf;

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

// ===========================================================================
// mika#2338 — unattested disposition escalates with its cause
// ===========================================================================

/// What mika-arch sees in production: the three arch skills are `always_on`, so the
/// union of all five declared lines is present on every turn.
fn arch_suffix_union() -> Vec<String> {
    [
        "Disposition: READY",
        "Disposition: ITERATE",
        "Disposition: ESCALATE",
        "Verdict: GROOMED",
        "Verdict: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn finding_prefixes() -> Vec<String> {
    (1..=3).map(|n| format!("F{n}:")).collect()
}

fn escalate_2338(text: &str, suffixes: &[String], prefixes: &[String]) -> Option<String> {
    escalate_unattested_disposition(
        text,
        suffixes,
        prefixes,
        3,
        2,
        review_anchor::AnchorMissReason::QuoteNotInBrief,
    )
}

fn last_non_empty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

#[test]
fn mika2338_first_pass_2_of_3_becomes_a_disposition_escalate_naming_the_cause() {
    let text = "A1: \"une citation\" — ok.\nA2: \"une autre\" — ok.\nA3: paraphrase.\n\nLe plan est sain.\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes())
        .expect("the union declares Disposition: ESCALATE");
    assert_eq!(last_non_empty_line(&out), "Disposition: ESCALATE");
    assert!(
        out.contains("F1: (BLOCKING) [mika-engine] review-anchor: attestation withheld"),
        "finding line must open with the prefix and the constant marker: {out}"
    );
    assert!(out.contains("anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief"));
    assert!(
        !out.contains("Disposition: READY"),
        "no READY may survive: {out}"
    );
    assert!(
        out.contains("Le plan est sain."),
        "the model's body is kept"
    );
}

#[test]
fn mika2338_second_pass_groomed_becomes_a_verdict_escalate_not_a_disposition() {
    let text = "A1: \"une citation\" — ok.\nA2: \"une autre\" — ok.\nA3: paraphrase.\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
    assert!(!out.contains("Verdict: GROOMED"));
    assert!(
        !out.contains("Disposition: ESCALATE"),
        "the family follows the withdrawn line, not the order of the list: {out}"
    );
}

#[test]
fn mika2338_undeclared_family_falls_back_to_none() {
    let only_dispositions: Vec<String> = [
        "Disposition: READY",
        "Disposition: ITERATE",
        "Disposition: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let text = "A1: x\n\nVerdict: GROOMED\n";
    assert_eq!(
        escalate_2338(text, &only_dispositions, &finding_prefixes()),
        None,
        "Verdict: ESCALATE is not declared — the caller withholds instead"
    );

    let only_ready = vec!["Disposition: READY".to_string()];
    let text = "A1: x\n\nDisposition: READY\n";
    assert_eq!(escalate_2338(text, &only_ready, &finding_prefixes()), None);
}

#[test]
fn mika2338_no_finding_prefixes_declared_defaults_to_f1() {
    let text = "A1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &[]).unwrap();
    assert!(
        out.contains("\nF1: (BLOCKING) [mika-engine] review-anchor:"),
        "{out}"
    );
}

#[test]
fn mika2338_a_disposition_cited_early_on_its_own_line_is_rewritten_too() {
    let text = "Le contrat dit :\nDisposition: READY\nquand le plan est sain.\n\nA1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert_eq!(out.matches("Disposition: ESCALATE").count(), 3, "{out}");
}

/// The case the withheld marker used to cover at tier 0 and that this path must cover
/// itself: `dispatch-lib` reads the FIRST `Disposition:` anywhere in the text.
#[test]
fn mika2338_an_inline_mention_of_the_disposition_is_rewritten() {
    let text = "Je confirme que ma réponse se termine par `Disposition: READY` comme demandé.\n\nA1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(
        out.contains("se termine par `Disposition: ESCALATE`"),
        "{out}"
    );

    let text = "Mon verdict est bien `Verdict: GROOMED`.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Verdict: GROOMED"), "{out}");
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

/// A READY quoted inline during a second-review turn is rewritten into ITS family's
/// ESCALATE, so `_parse_disposition`'s tier 1a cannot read READY out of it either.
#[test]
fn mika2338_a_foreign_family_mention_is_rewritten_into_its_own_escalate() {
    let text = "La première passe disait Disposition: READY.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(out.contains("disait Disposition: ESCALATE."), "{out}");
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

/// The cross-family fallback inside the rewrite loop: a declared non-terminal line whose
/// own family has no declared ESCALATE is rewritten into the WITHDRAWN line's ESCALATE,
/// so no declared verdict keyword survives. Unreachable with the shipped always_on arch
/// manifests (their union always carries both families); live code for a future
/// single-family verdict producer.
#[test]
fn mika2338_a_line_whose_family_has_no_escalate_is_rewritten_into_the_withdrawn_family() {
    // Verdict family complete; Disposition family declares READY but no ESCALATE.
    let partial: Vec<String> = [
        "Disposition: READY",
        "Verdict: GROOMED",
        "Verdict: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let text = "La première passe disait Disposition: READY.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &partial, &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(
        out.contains("disait Verdict: ESCALATE."),
        "the READY mention falls back to the withdrawn line's family: {out}"
    );
    assert!(
        !out.contains("Disposition: ESCALATE"),
        "undeclared, must not be emitted: {out}"
    );
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

#[test]
fn mika2338_the_finding_line_never_carries_a_declared_line_in_clear() {
    for reason in [
        review_anchor::AnchorMissReason::NoAnchorLine,
        review_anchor::AnchorMissReason::QuoteTooShort,
        review_anchor::AnchorMissReason::QuoteNotInBrief,
        review_anchor::AnchorMissReason::OverlappingRegions,
        review_anchor::AnchorMissReason::TooFewAnchors,
    ] {
        let out = escalate_unattested_disposition(
            "A1: x\n\nDisposition: READY\n",
            &arch_suffix_union(),
            &finding_prefixes(),
            1,
            0,
            reason,
        )
        .unwrap();
        let finding = out
            .lines()
            .find(|l| l.starts_with("F1: "))
            .expect("finding line present");
        assert!(
            !finding.contains("READY") && !finding.contains("GROOMED"),
            "{finding}"
        );
        for req in arch_suffix_union() {
            assert!(!finding.contains(&req), "{finding} carries {req}");
        }
    }
}

// ===========================================================================
// mika#2794 — the corrective re-prompt names the faulty anchor
// ===========================================================================

fn outcome(
    label: &str,
    excerpt: &str,
    refused: Option<review_anchor::AnchorMissReason>,
) -> review_anchor::AnchorOutcome {
    review_anchor::AnchorOutcome {
        label: label.to_string(),
        excerpt: excerpt.to_string(),
        refused,
    }
}

/// The measured shape of mika#2794: `A1` quotes the ticket body, `A2` and `A3` quote the
/// brief. The diagnosis must name all three — the refused one so it is rewritten, the two
/// accepted so they are **not** — and say where a wrong quote usually comes from.
#[test]
fn mika2794_the_diagnosis_names_the_refused_anchor_and_the_accepted_ones() {
    let per_anchor = vec![
        outcome(
            "A1:",
            "Le reaper groom introduit par mika#2750 (PR mika#2754)…",
            Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
        ),
        outcome("A2:", "sur l'exhaustivité du tracé", None),
        outcome("A3:", "Je n'ai pas fixé N pour le seuil", None),
    ];
    // mika#3003 — aucune attribution : ce test mesure la forme pré-mika#3003
    // du diagnostic, et c'est précisément ce qu'une liste vide doit rendre.
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    assert!(
        diagnosis.contains("Anchors accepted: A2, A3"),
        "the accepted anchors are named so the correction stays local: {diagnosis}"
    );
    assert!(
        diagnosis.contains("do not rewrite them"),
        "naming them is useless without the instruction to keep them: {diagnosis}"
    );
    assert!(
        diagnosis
            .contains("A1: \"Le reaper groom introduit par mika#2750 (PR mika#2754)…\" — refused:"),
        "the refused anchor is named by label AND by what it said: {diagnosis}"
    );
    assert!(
        !diagnosis.contains("A2: \"") && !diagnosis.contains("A3: \""),
        "an accepted anchor is listed, never quoted back as a failure: {diagnosis}"
    );
    assert!(
        diagnosis.contains("a document you read with a tool"),
        "the measured cause — a tool-read document, not this turn's brief: {diagnosis}"
    );
}

/// Negative control of the origin sentence, and the reason it is gated. Anchors that all
/// quote the **same region of the right brief** have not quoted another document; telling
/// the model it did would send it looking for a brief it already quoted.
#[test]
fn mika2794_an_overlap_refusal_does_not_blame_another_document() {
    let per_anchor = vec![
        outcome("A1:", "re-résoudre le token avant chaque cycle", None),
        outcome(
            "A2:",
            "re-résoudre le token avant chaque cycle",
            Some(review_anchor::AnchorMissReason::OverlappingRegions),
        ),
    ];
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    assert!(diagnosis.contains("Anchors accepted: A1"), "{diagnosis}");
    assert!(diagnosis.contains("A2: \"re-résoudre"), "{diagnosis}");
    assert!(
        !diagnosis.contains("a document you read with a tool"),
        "the origin sentence is gated on QuoteNotInBrief: {diagnosis}"
    );
}

/// The other negative control: the matcher's two early returns report no per-line fate, and
/// the corrective re-prompt must then be character for character its pre-mika#2794 self.
/// `None` is the whole mechanism — the insertion point is a `{}` filled with `""`.
#[test]
fn mika2794_an_empty_fate_list_leaves_the_reprompt_untouched() {
    assert_eq!(review_anchor::format_anchor_diagnosis(&[], &[]), None);
}

/// The paragraph is bounded. Five refused anchors name three — a wall of ten lines is a
/// wall the model skims, and the measured shape refuses one of three.
#[test]
fn mika2794_the_diagnosis_names_at_most_three_refused_anchors() {
    let per_anchor: Vec<review_anchor::AnchorOutcome> = (1..=5)
        .map(|n| {
            outcome(
                &format!("A{n}:"),
                "une ligne qui ne cite pas le brief",
                Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
            )
        })
        .collect();
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    let named = (1..=5)
        .filter(|n| diagnosis.contains(&format!("A{n}: \"")))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        vec![1, 2, 3],
        "the first MAX_DIAGNOSED_ANCHORS in body order, and only those: {diagnosis}"
    );
    assert!(
        !diagnosis.contains("Anchors accepted:"),
        "nothing was accepted, so nothing is claimed to have been: {diagnosis}"
    );
}

/// The instrument's two fields (mika#2794 D3). Empty is an empty string, never an absent
/// field: absence means a binary older than this change, not an absence of refusal.
#[test]
fn mika2794_the_label_fields_are_empty_strings_not_absent() {
    let (accepted, refused) = anchor_label_fields(&[
        outcome(
            "A1:",
            "x",
            Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
        ),
        outcome("A2:", "y", None),
        outcome("A3:", "z", None),
    ]);
    assert_eq!(refused, "A1");
    assert_eq!(accepted, "A2,A3");

    let (accepted, refused) = anchor_label_fields(&[]);
    assert_eq!((accepted.as_str(), refused.as_str()), ("", ""));
}

// ===========================================================================
// mika#2296 — reasoning-budget-exhaustion predicate (M2 / T3)
// ===========================================================================

/// The founding signature: MaxTokens, a full output budget billed, nothing
/// visible. This is the arch pass `8b623724` — 123 s, 8192 output tokens,
/// empty content, no `Disposition:` for dispatch-lib to parse.
#[test]
fn mika2296_maxtokens_with_empty_text_and_billed_output_is_budget_exhaustion() {
    assert!(is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "",
        8192
    ));
    // Whitespace-only is the same failure wearing a newline.
    assert!(is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "\n  \n",
        8192
    ));
}

/// A turn that hit the ceiling *while writing* is truncated output, not a
/// starved verdict — the operator's remedy differs, so the signal must not
/// conflate them.
#[test]
fn mika2296_maxtokens_with_visible_text_is_not_budget_exhaustion() {
    assert!(!is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "Disposition: READY",
        8192
    ));
}

/// Without the `output_tokens > 0` term, a provider that returned nothing at
/// all would read as a budget problem and send the operator to raise a
/// ceiling that was never reached.
#[test]
fn mika2296_zero_output_tokens_is_an_empty_response_not_a_starved_budget() {
    assert!(!is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "",
        0
    ));
}

/// The other stop reasons keep their own meanings; an empty `EndTurn` is a
/// different defect with a different remedy.
#[test]
fn mika2296_other_stop_reasons_never_fire_the_budget_signal() {
    for stop_reason in [
        LlmStopReason::EndTurn,
        LlmStopReason::ToolUse,
        LlmStopReason::ContentFilter,
    ] {
        assert!(
            !is_reasoning_budget_exhausted(stop_reason, "", 8192),
            "{stop_reason:?} must not be reported as reasoning-budget exhaustion"
        );
    }
}

// ===========================================================================
// mika#1324 — Structural tool filter (CI webhook → no dispatch tools)
// ===========================================================================

fn tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("test stub for {name}"),
        input_schema: serde_json::json!({}),
    }
}

fn full_tool_set() -> Vec<ToolDefinition> {
    vec![
        tool_def("run_claude_pilot"),
        tool_def("run_claude_pilot_groom"),
        tool_def("send_message"),
        tool_def("run_gh"),
        tool_def("update_task_status"),
    ]
}

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

// -- check_onboarding tests --

#[tokio::test]
async fn test_check_onboarding_true_when_no_core_memory() {
    let db = test_async_db();
    // Fresh DB has no core memory — should be onboarding
    assert!(check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_true_when_default_value() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    // Seeded with defaults — still onboarding
    assert!(check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_false_when_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer")
        .await
        .unwrap();
    assert!(!check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_true_even_when_other_sections_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("self_model", "Custom self model")
        .await
        .unwrap();
    assert!(check_onboarding(&db).await);
}

// -- seed_user_person tests --

#[tokio::test]
async fn test_seed_user_person_after_onboarding() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer at Senara Solutions")
        .await
        .unwrap();

    seed_user_person(&db).await.unwrap();

    let person = db.get_person("Sam").await.unwrap();
    assert!(person.is_some());
    let person = person.unwrap();
    assert_eq!(person.relationship.as_deref(), Some("The user"));
}

#[tokio::test]
async fn test_seed_user_person_skips_when_default() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    // user_summary is still at default

    seed_user_person(&db).await.unwrap();

    let people = db.list_people().await.unwrap();
    assert!(people.is_empty());
}

#[tokio::test]
async fn test_seed_user_person_skips_when_already_exists() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer")
        .await
        .unwrap();
    // Pre-create the person
    db.upsert_person("Sam", Some("Friend"), None).await.unwrap();

    seed_user_person(&db).await.unwrap();

    // Relationship should NOT be overwritten
    let person = db.get_person("Sam").await.unwrap().unwrap();
    assert_eq!(person.relationship.as_deref(), Some("Friend"));
}

#[tokio::test]
async fn test_seed_user_person_extracts_name_before_comma() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Alice Johnson, product manager")
        .await
        .unwrap();

    seed_user_person(&db).await.unwrap();

    let person = db.get_person("Alice Johnson").await.unwrap();
    assert!(person.is_some());
}

// -- Skill helper tests --

fn make_skill_entry(name: &str, timeout: u64, tool_names: &[&str]) -> SkillEntry {
    make_skill_entry_with_constraints(name, timeout, tool_names, &[])
}

fn make_skill_entry_with_constraints(
    name: &str,
    timeout: u64,
    tool_names: &[&str],
    required_tools: &[&str],
) -> SkillEntry {
    use crate::skills::manifest::Constraints;
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: timeout,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![name.to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: std::collections::HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: vec![name.to_lowercase()],
        prompt_snippet: String::new(),
        skill_tools: tool_names
            .iter()
            .map(|tn| ResolvedSkillTool {
                definition: ToolDefinition {
                    name: tn.to_string(),
                    description: format!("{tn} tool"),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                handler: ToolHandler::Builtin {
                    function: tn.to_string(),
                },
                skill_dir: PathBuf::from(format!("/skills/{name}")),
            })
            .collect(),
        enabled: true,
        has_override: false,
        provider_overrides: std::collections::HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: std::collections::HashMap::new(),
    }
}

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

#[test]
fn test_inject_skills_skips_empty_snippets() {
    let tools = ToolRegistry::new();
    let entry = make_skill_entry("quiet", 30, &["quiet_tool"]);
    // prompt_snippet is empty by default from make_skill_entry
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base.".to_string();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    // Should NOT add skill context section when snippet is empty
    assert!(!system.contains("quiet Skill"));
    assert_eq!(system, "Base.");
    // No variant info when prompt is empty
    assert!(variant.is_none());
}

#[test]
fn test_inject_skills_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt for search.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    // No model variant — should fall back to root
    let no_ctx = HashMap::new();
    let (_defs, _variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "groq",
        "llama-3.3-70b-versatile",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt for search."));
}

#[test]
fn test_inject_skills_no_prompt_at_all() {
    let tools = ToolRegistry::new();
    let entry = make_skill_entry("quiet", 30, &["quiet_tool"]);
    // No root prompt, no provider prompts
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base.".to_string();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    // Should NOT add any skill context section
    assert_eq!(system, "Base.");
    assert!(variant.is_none());
}

#[test]
fn test_inject_skills_uses_model_prompt() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        "Sonnet-specific prompt.".to_string(),
    );
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Sonnet-specific prompt."));
    assert!(!system.contains("Root prompt."));
    // Should record hand-authored model variant
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(
        variant_json.get("search").unwrap(),
        "hand_authored_model:anthropic/claude-sonnet-4-6"
    );
}

#[test]
fn test_inject_skills_model_no_match_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        "Sonnet prompt.".to_string(),
    );
    // No model variant for claude-opus-4 — should fall back to root
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-opus-4",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt."));
    assert!(!system.contains("Sonnet prompt."));
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(variant_json.get("search").unwrap(), "base");
}

#[test]
fn test_inject_skills_model_falls_back_to_root() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    // No provider or model variant for groq/llama
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    let no_ctx = HashMap::new();
    let (_defs, _variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "groq",
        "llama-3.3-70b-versatile",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("Root prompt."));
}

#[test]
fn test_inject_skills_model_with_slash() {
    let tools = ToolRegistry::new();
    let mut entry = make_skill_entry("search", 30, &[]);
    entry.prompt_snippet = "Root prompt.".to_string();
    entry.model_prompts_mut().insert(
        "openrouter/anthropic--claude-sonnet-4".to_string(),
        "OpenRouter model prompt.".to_string(),
    );
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = String::new();

    // Model name contains a slash — sanitize_model_dir_name should match
    let no_ctx = HashMap::new();
    let (_defs, variant, _) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "openrouter",
        "anthropic/claude-sonnet-4",
        &no_ctx,
        "mika-test",
        &[],
        None,
        false,
    );

    assert!(system.contains("OpenRouter model prompt."));
    assert!(!system.contains("Root prompt."));
    let variant_json: HashMap<String, String> =
        serde_json::from_str(variant.as_ref().unwrap()).unwrap();
    assert_eq!(
        variant_json.get("search").unwrap(),
        "hand_authored_model:openrouter/anthropic--claude-sonnet-4"
    );
}

#[test]
fn test_max_skill_timeout_uses_model_override() {
    let mut s1 = make_skill_entry("search", 30, &[]);
    s1.provider_overrides.insert(
        "anthropic".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(90),
            max_prompt_size: None,
        },
    );
    s1.model_overrides.insert(
        "anthropic/claude-sonnet-4-6".to_string(),
        crate::skills::manifest::ProviderSkillFields {
            timeout_secs: Some(120),
            max_prompt_size: None,
        },
    );
    let matched: Vec<&SkillEntry> = vec![&s1];
    // With model override, should use model
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-sonnet-4-6"),
        120
    );
    // Without model override, should use provider
    assert_eq!(
        max_skill_timeout(&matched, "anthropic", "claude-opus-4"),
        90
    );
    // Without provider or model, should use root
    assert_eq!(max_skill_timeout(&matched, "groq", "llama"), 30);
}

#[test]
fn test_format_step_exceeded_fallback_non_zero_exit_old_format() {
    // Backward compat: old metadata had success: true with non_zero_exit: true
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "shell_exec".to_string(),
        input_summary: "grep foo".to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: true,
        non_zero_exit: true,
    }];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(
        result.contains("- shell_exec (non-zero exit)"),
        "expected non-zero exit status in: {result}"
    );
}

#[test]
fn test_format_step_exceeded_fallback_non_zero_exit_new_format() {
    // New format: success is false when non_zero_exit is true
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "shell_exec".to_string(),
        input_summary: "grep foo".to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(
        result.contains("- shell_exec (non-zero exit)"),
        "expected non-zero exit status in: {result}"
    );
}

// -- DB metadata integration tests --

#[tokio::test]
async fn test_save_and_load_message_with_metadata() {
    let db = test_async_db();
    let metadata = r#"{"tool_calls":[{"step":0,"name":"search_memory","input_summary":"q","output_summary":"found","success":true}]}"#;
    db.save_message_with_metadata(
        "test-session",
        "assistant",
        "I searched your memory.",
        Some(metadata),
        None,
        false,
    )
    .await
    .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "assistant");
    assert_eq!(messages[0].content, "I searched your memory.");
    assert_eq!(messages[0].metadata.as_deref(), Some(metadata));
}

#[tokio::test]
async fn test_save_message_without_metadata_loads_as_none() {
    let db = test_async_db();
    db.save_message("test-session", "user", "Hello", None)
        .await
        .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].metadata.is_none());
}

#[tokio::test]
async fn test_save_message_with_null_metadata() {
    let db = test_async_db();
    db.save_message_with_metadata(
        "test-session",
        "assistant",
        "No tools used.",
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let messages = db.load_recent_messages(10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].metadata.is_none());
}

// -- strip_prior_images tests --

#[test]
fn test_strip_prior_images_removes_image_blocks() {
    let mut messages = vec![
        // Prior turn: user message with tool results containing images
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_1".to_string(),
                content: LlmToolResultContent::Blocks(vec![
                    LlmToolResultBlock::Text("Screenshot taken.".to_string()),
                    LlmToolResultBlock::Image(LlmImage {
                        media_type: "image/png".to_string(),
                        data: "iVBORw0KGgo=".to_string(),
                    }),
                ]),
                is_error: false,
            }]),
        },
        // Prior turn: assistant response
        LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Text("I see a desktop.".to_string()),
        },
        // Current turn: new tool results (should be preserved)
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_2".to_string(),
                content: LlmToolResultContent::Blocks(vec![
                    LlmToolResultBlock::Text("New screenshot.".to_string()),
                    LlmToolResultBlock::Image(LlmImage {
                        media_type: "image/png".to_string(),
                        data: "iVBORw0KGgo=".to_string(),
                    }),
                ]),
                is_error: false,
            }]),
        },
    ];

    strip_prior_images(&mut messages);

    // First message should have images stripped
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        if let LlmContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(
                matches!(content, LlmToolResultContent::Text(t) if t.contains("Screenshot taken.") && t.contains("omitted"))
            );
        } else {
            panic!("expected ToolResult");
        }
    } else {
        panic!("expected Blocks");
    }

    // Last message should still have images
    if let LlmContent::Blocks(blocks) = &messages[2].content {
        if let LlmContentBlock::ToolResult { content, .. } = &blocks[0] {
            assert!(matches!(content, LlmToolResultContent::Blocks(_)));
        } else {
            panic!("expected ToolResult");
        }
    } else {
        panic!("expected Blocks");
    }
}

#[test]
fn test_strip_prior_images_preserves_text_only() {
    let mut messages = vec![
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tu_1".to_string(),
                content: LlmToolResultContent::Text("just text".to_string()),
                is_error: false,
            }]),
        },
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("current turn".to_string()),
        },
    ];

    strip_prior_images(&mut messages);

    // Text-only tool result should be unchanged
    if let LlmContent::Blocks(blocks) = &messages[0].content
        && let LlmContentBlock::ToolResult { content, .. } = &blocks[0]
    {
        assert!(matches!(content, LlmToolResultContent::Text(t) if t == "just text"));
    }
}

#[test]
fn test_strip_prior_images_removes_user_attached_images() {
    let mut messages = vec![
        // Prior turn: user message with text and an attached image
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Text("What is in this picture?".to_string()),
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgo=".to_string(),
                }),
            ]),
        },
        // Assistant response
        LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Text("I see a cat.".to_string()),
        },
        // Current turn: user message with a new image (should be preserved)
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Text("And this one?".to_string()),
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/jpeg".to_string(),
                    data: "/9j/4AAQ=".to_string(),
                }),
            ]),
        },
    ];

    strip_prior_images(&mut messages);

    // First message: image should be replaced with placeholder text
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        assert_eq!(blocks.len(), 2);
        assert!(
            matches!(&blocks[0], LlmContentBlock::Text(text) if text == "What is in this picture?")
        );
        assert!(
            matches!(&blocks[1], LlmContentBlock::Text(text) if text == "[user image from previous turn omitted]")
        );
    } else {
        panic!("expected Blocks for first message");
    }

    // Last message: image should still be intact
    if let LlmContent::Blocks(blocks) = &messages[2].content {
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], LlmContentBlock::Text(_)));
        assert!(matches!(&blocks[1], LlmContentBlock::Image(_)));
    } else {
        panic!("expected Blocks for last message");
    }
}

#[test]
fn test_strip_prior_images_no_mutation_without_images() {
    let mut messages = vec![
        // Prior turn: user message with only text blocks
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![LlmContentBlock::Text("Hello".to_string())]),
        },
        // Current turn
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("Current".to_string()),
        },
    ];

    strip_prior_images(&mut messages);

    // Text-only blocks should remain unchanged
    if let LlmContent::Blocks(blocks) = &messages[0].content {
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], LlmContentBlock::Text(text) if text == "Hello"));
    } else {
        panic!("expected Blocks");
    }
}

// -- format_step_exceeded_fallback tests --

#[test]
fn test_format_step_exceeded_fallback_empty_summaries() {
    let result = format_step_exceeded_fallback(&[]);
    assert!(result.contains("I ran out of steps"));
    assert!(result.contains("You can ask me to continue"));
    // No bullet points when empty
    assert!(!result.contains("- "));
}

#[test]
fn test_format_step_exceeded_fallback_few_summaries() {
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "search_memory".to_string(),
            input_summary: "query".to_string(),
            output_summary: "found 3 results".to_string(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "run_shell".to_string(),
            input_summary: "ls".to_string(),
            output_summary: "error".to_string(),
            success: false,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "read_file".to_string(),
            input_summary: "/tmp/test".to_string(),
            output_summary: "contents".to_string(),
            success: true,
            non_zero_exit: false,
        },
    ];
    let result = format_step_exceeded_fallback(&summaries);
    assert!(result.contains("- search_memory (done)"));
    assert!(result.contains("- run_shell (failed)"));
    assert!(result.contains("- read_file (done)"));
    assert!(result.contains("You can ask me to continue"));
}

#[test]
fn test_format_step_exceeded_fallback_truncates_to_last_five() {
    let summaries: Vec<ToolCallSummary> = (0..10)
        .map(|i| ToolCallSummary {
            step: i,
            name: format!("tool_{i}"),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        })
        .collect();
    let result = format_step_exceeded_fallback(&summaries);
    // Should only show last 5 (tool_5 through tool_9)
    assert!(!result.contains("tool_0"));
    assert!(!result.contains("tool_4"));
    assert!(result.contains("tool_5"));
    assert!(result.contains("tool_9"));
}

// -- format_callback_framing tests --

#[test]
fn test_format_callback_framing_completed() {
    let result =
        format_callback_framing("analyze_code", "task-123", None, "Analysis complete", false);
    assert!(result.contains("A background task has completed."));
    assert!(result.contains("analyze_code"));
    assert!(result.contains("task-123"));
    assert!(result.contains("Analysis complete"));
    assert!(result.contains("<callback_result trust=\"untrusted\">"));
    assert!(!result.contains("FAILED"));
}

#[test]
fn test_format_callback_framing_failed() {
    let result = format_callback_framing(
        "run_pilot",
        "task-456",
        None,
        "Process exited with code 128: fatal error",
        true,
    );
    assert!(result.contains("A background task has FAILED."));
    assert!(result.contains("run_pilot"));
    assert!(result.contains("task-456"));
    assert!(result.contains("Process exited with code 128: fatal error"));
    assert!(result.contains("<callback_result trust=\"untrusted\">"));
    assert!(!result.contains("has completed"));
}

#[test]
fn test_format_callback_framing_short_result_not_truncated() {
    let short = "a".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES);
    let result = format_callback_framing("task", "id-1", None, &short, false);
    assert!(result.contains(&short));
    assert!(!result.contains("[truncated"));
}

#[test]
fn test_format_callback_framing_long_result_truncated() {
    let long = "x".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES + 5000);
    let result = format_callback_framing("task", "id-2", None, &long, false);
    assert!(!result.contains(&long));
    assert!(result.contains("[truncated — full result available in task logs]"));
    // The truncated content should be present (up to the cut boundary)
    let suffix_len = "\n...\n[truncated — full result available in task logs]".len();
    let prefix = &"x".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len);
    assert!(result.contains(prefix));
}

#[test]
fn test_format_callback_framing_truncation_utf8_safe() {
    // Place a 4-byte emoji so it straddles the cut point, forcing the
    // char-boundary walk-back loop to execute.
    // cut = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len ≈ 10_185
    // Emoji at byte (cut-1) spans (cut-1)..(cut+2), so cut lands mid-emoji.
    let suffix_len = "\n...\n[truncated — full result available in task logs]".len();
    let cut = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len;
    let mut s = "a".repeat(cut - 1); // one byte before the cut point
    s.push('🦀'); // 4-byte char that straddles the cut boundary
    // Pad with enough trailing data to exceed crate::planning::policy::CALLBACK_RESULT_MAX_BYTES
    let pad = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - s.len() + 1;
    s.push_str(&"z".repeat(pad));
    assert!(s.len() > crate::planning::policy::CALLBACK_RESULT_MAX_BYTES);
    let result = format_callback_framing("task", "id-3", None, &s, true);
    assert!(result.contains("[truncated"));
    // The emoji should NOT be in the output (it was at the boundary)
    assert!(!result.contains('🦀'));
    // Content up to the emoji should be preserved
    assert!(result.contains(&"a".repeat(cut - 1)));
}

#[test]
fn test_format_callback_framing_includes_grounding_instruction() {
    let result = format_callback_framing(
        "build_code",
        "task-789",
        None,
        "Compilation succeeded",
        false,
    );
    assert!(result.contains("Report only what this result explicitly states"));
    assert!(result.contains("Do not infer the state of any system, artifact, or process"));
}

// -- Callback trigger context tests (#313) --

#[test]
fn test_callback_trigger_generic_framing_for_all_labels() {
    // All callbacks — including claude-pilot — get the same generic framing.
    // Workflow-specific behavior is driven by active skill prompts. See #313.
    for label in [
        "long_running:run_claude_pilot",
        "long_running:run_shell",
        "long_running:custom_task",
    ] {
        let ctx = build_callback_trigger_context(label, "task-001", None, "Result text", false);
        assert!(
            ctx.contains("Follow the workflow defined by your active skills"),
            "label={label}: missing generic skill delegation instruction"
        );
        assert!(
            ctx.contains("NEVER extrapolate to downstream states"),
            "label={label}: missing grounding instruction"
        );
    }
}

#[test]
fn test_callback_trigger_failed_uses_generic_framing() {
    let ctx = build_callback_trigger_context(
        "long_running:run_shell",
        "task-002",
        None,
        "Script failed",
        true,
    );
    // Generic framing for failed callbacks
    assert!(ctx.contains("Follow the workflow defined by your active skills"));
    // The base framing indicates FAILED
    assert!(ctx.contains("FAILED"));
}

#[test]
fn test_callback_framing_with_parent_task_id() {
    let ctx = build_callback_trigger_context(
        "long_running:run_claude_pilot",
        "task-003",
        Some("wi-parent-uuid-123"),
        "PR created: https://github.com/example/repo/pull/42",
        false,
    );
    assert!(ctx.contains("Parent task: wi-parent-uuid-123"));
    assert!(ctx.contains("Task: 'long_running:run_claude_pilot' (ID: task-003)"));
}

#[test]
fn test_callback_framing_without_parent_task_id() {
    let ctx = build_callback_trigger_context(
        "long_running:run_shell",
        "task-004",
        None,
        "Script completed",
        false,
    );
    assert!(!ctx.contains("Parent task"));
    assert!(ctx.contains("Task: 'long_running:run_shell' (ID: task-004)"));
}

#[test]
fn test_callback_framing_parent_task_id_with_failed() {
    let ctx = build_callback_trigger_context(
        "long_running:run_claude_pilot",
        "task-005",
        Some("wi-parent-uuid-456"),
        "Error: cargo build failed",
        true,
    );
    // Parent line still present even when the callback failed
    assert!(ctx.contains("Parent task: wi-parent-uuid-456"));
    assert!(ctx.contains("FAILED"));
}

// -- Task health injection guard tests (#314) --

#[test]
fn test_task_health_guard_includes_heartbeat() {
    let trigger = SilentTrigger::Heartbeat;
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        should_inject,
        "Heartbeat trigger should receive task health"
    );
}

#[test]
fn test_task_health_guard_includes_callback() {
    let trigger = SilentTrigger::Callback {
        task_id: "task-100".to_string(),
        label: "long_running:run_claude_pilot".to_string(),
        result: "PR created".to_string(),
        failed: false,
        parent_task_id: None,
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(should_inject, "Callback trigger should receive task health");
}

#[test]
fn test_task_health_guard_includes_failed_callback() {
    let trigger = SilentTrigger::Callback {
        task_id: "task-101".to_string(),
        label: "long_running:run_shell".to_string(),
        result: "Script failed".to_string(),
        failed: true,
        parent_task_id: Some("wi-parent-101".to_string()),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        should_inject,
        "Failed callback trigger should also receive task health"
    );
}

#[test]
fn test_task_health_guard_excludes_reflection() {
    let trigger = SilentTrigger::Reflection;
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        !should_inject,
        "Reflection trigger should NOT receive task health"
    );
}

#[test]
fn test_task_health_guard_excludes_skill_run() {
    let trigger = SilentTrigger::SkillRun {
        skill_name: "web-search".to_string(),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        !should_inject,
        "SkillRun trigger should NOT receive task health"
    );
}

#[test]
fn test_task_health_guard_includes_reminder() {
    let trigger = SilentTrigger::Reminder {
        task_id: "task-200".to_string(),
        message: "Check CI status".to_string(),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(should_inject, "Reminder trigger should receive task health");
}

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

// mika#3004 — ouvert au frère `pr_review_groom`, et pas plus haut.
pub(in crate::agent_loop) fn make_resolved_skill_tool(name: &str) -> ResolvedSkillTool {
    ResolvedSkillTool {
        definition: ToolDefinition {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({"type": "object"}),
        },
        handler: ToolHandler::Builtin {
            function: name.to_string(),
        },
        skill_dir: PathBuf::from("/skills/test"),
    }
}

/// Create a ToolRegistry with a single dummy builtin tool named `name`.
pub(in crate::agent_loop) fn make_registry_with_tool(name: &'static str) -> ToolRegistry {
    struct DummyBuiltin(&'static str);
    #[async_trait::async_trait]
    impl crate::tools::Tool for DummyBuiltin {
        fn name(&self) -> &str {
            self.0
        }
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.to_string(),
                description: format!("{} tool", self.0),
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
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(DummyBuiltin(name)));
    registry
}

/// mika#2757 — le tableau d'outils servi (`enabled_tool_names`, #862) pour un
/// cas qui ne mesure pas le désarmement : tout ce qui est exigé est servi.
pub(in crate::agent_loop) fn served(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

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

/// Le message mesuré du 2026-10-17 ≈ 01:01Z, reconstitué sur la forme que
/// `mika_gateway::github::format_event_text` émet pour `issues.closed` :
/// préfixe, référence, tiret cadratin, titre, puis le corps cité.
///
/// **Deux propriétés sont load-bearing, et seules elles.** Le préfixe
/// `[GitHub] Issue closed:` met le tour dans le domaine Fallthrough (ni
/// ready-label, ni PR, ni check-suite), donc
/// `webhook_no_unauthorized_dispatch` y interdit tout dispatch. Et le mot
/// `groom` est dans le **corps cité** — ni dans le titre, ni dans un slug de
/// branche : le masque du vol 1 (mika#2755,
/// `github_event_format::mask_cited_branch`) ne lit que le marqueur
/// `(branch: …)` de la première ligne, donc il ne le couvre pas. C'est
/// exactement la seconde occurrence que ce vol doit couvrir.
pub(in crate::agent_loop) const TICKET_2750_CLOSED_MSG: &str = "[GitHub] Issue closed: senara-solutions/mika#2750 — un dispatch ready-label qui échoue\n\nLe dispatch auto-groom s'est interrompu avant d'ouvrir la session.\n";

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

// -- resolve_skill_llm_override tests (#463) --

fn make_skill_entry_with_llm(
    name: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> SkillEntry {
    use crate::skills::manifest::LlmOverride;
    let mut entry = make_skill_entry(name, 30, &[]);
    entry.manifest.llm = LlmOverride {
        provider: provider.map(String::from),
        model: model.map(String::from),
        from_db_override: false,
    };
    entry
}

/// Like `make_skill_entry_with_llm` but marks the LLM override as DB-sourced
/// (simulates `apply_overrides()` having written `skill_overrides` DB rows).
fn make_skill_entry_with_db_llm(
    name: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> SkillEntry {
    use crate::skills::manifest::LlmOverride;
    let mut entry = make_skill_entry(name, 30, &[]);
    entry.manifest.llm = LlmOverride {
        provider: provider.map(String::from),
        model: model.map(String::from),
        from_db_override: true,
    };
    entry
}

#[test]
fn test_resolve_skill_llm_override_returns_none_for_empty_matched() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("anthropic")
        .model_name("claude-sonnet-4-6")
        .build();
    let matched: Vec<MatchedSkill<'_>> = vec![];
    assert!(resolve_skill_llm_override(&matched, None, &mock, false).is_none());
}

#[test]
fn test_resolve_skill_llm_override_ignores_always_on_skills() {
    // always_on skill with [llm] should NOT impose override (#463)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "always_on skills should not impose [llm] override"
    );
}

#[test]
fn test_resolve_skill_llm_override_ignores_dependency_skills() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm("dev-pilot", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Dependency,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "dependency skills should not impose [llm] override"
    );
}

#[test]
fn test_resolve_skill_llm_override_mixed_reasons_only_keyword_considered() {
    // always_on skill with [llm] + keyword skill without [llm] → no override
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let s2 = make_skill_entry("skill-review", 30, &["review_skill"]);
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
    // skill-review has no [llm], self-dev is AlwaysOn → no override
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "only keyword-matched skills with [llm] should produce an override"
    );
}

#[test]
fn test_resolve_skill_llm_override_keyword_match_on_always_on_skill_applies() {
    // When an always_on skill is matched via keyword, its [llm] IS considered
    // (MatchReason is Keyword when keyword hit on an always_on skill)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword, // keyword hit on always_on skill → Keyword wins
    }];
    // Should attempt override — will return None because Settings is None,
    // but the important thing is it doesn't return None at the "no overrides" early exit.
    // We verify by checking overrides were collected (Settings absence causes the fallback path).
    let result = resolve_skill_llm_override(&matched, None, &mock, false);
    // Without Settings, can't construct provider — returns None via the "requires Settings" path.
    // But the function got past the "overrides.is_empty()" check, proving keyword was considered.
    assert!(result.is_none()); // Expected: Settings=None means it can't construct
}

#[test]
fn test_resolve_skill_llm_override_same_provider_short_circuit() {
    // Keyword skill with [llm] matching the active provider → returns None (no-op)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("qwen/qwen3-coder-plus")
        .build();
    let s1 = make_skill_entry_with_llm(
        "qa-review",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "same provider+model should short-circuit to None"
    );
}

#[test]
fn test_resolve_skill_llm_override_always_on_with_db_override_applies() {
    // AlwaysOn skill with DB-sourced LLM override SHOULD impose override (mika#1011).
    // The operator set this via `mika skills llm set` — it represents explicit intent.
    // Simulates: mika-dev's self-dev skill with DB override to anthropic/claude-sonnet-4-6
    // on a webhook turn where self-dev matches as AlwaysOn (no keyword hit).
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("moonshotai")
        .model_name("kimi-k2.5")
        .build();
    let s1 = make_skill_entry_with_db_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    // Should attempt override — will return None because Settings is None,
    // but the important thing is it gets past the "overrides.is_empty()" check.
    // Without Settings, can't construct provider — returns None via the "requires Settings" path.
    let result = resolve_skill_llm_override(&matched, None, &mock, false);
    assert!(result.is_none()); // Expected: Settings=None means it can't construct
    // The real verification is that this does NOT return None at the early
    // "overrides.is_empty()" exit — same pattern as the keyword test above.
}

#[test]
fn test_resolve_skill_llm_override_always_on_without_db_override_still_ignored() {
    // Regression guard for #463: AlwaysOn skill with [llm] from skill.toml
    // (from_db_override = false) should still NOT impose override.
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    // Verify from_db_override is false (developer-time source)
    assert!(!s1.manifest.llm.from_db_override);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "AlwaysOn with non-DB [llm] should not impose override (#463 regression guard)"
    );
}

#[test]
fn test_resolve_skill_llm_override_dependency_with_db_override_still_ignored() {
    // Dependency-matched skills should NEVER impose override, even with DB source.
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 =
        make_skill_entry_with_db_llm("dev-pilot", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Dependency,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "dependency skills with DB override should still not impose [llm] override"
    );
}

/// mika#1217 / mika#1011 — override-scope contract for SilentTrigger::Callback
/// and SilentTrigger::DeferredDispatch.
///
/// Silent-mode trigger semantics for AlwaysOn + DB-sourced LLM overrides: the
/// matched-skill set returned by `callback_safe_skills()` is wrapped with
/// `MatchReason::AlwaysOn`. Under the mika#1011 carve-out, an AlwaysOn skill
/// with `from_db_override = true` MUST qualify for override resolution —
/// otherwise the autonomous-loop callback turn runs on the agent's base
/// model instead of the operator-set per-skill override (e.g., mika-dev
/// base = kimi-k2.5, self-dev override = sonnet-4-6).
///
/// This test exercises both `SilentTrigger::Callback` and
/// `SilentTrigger::DeferredDispatch` shapes via the matched-skill construction
/// they share (`callback_safe_skills`). The carve-out fires identically.
///
/// Note (mika#1217 F3): `run_silent_inner` does not currently invoke
/// `resolve_skill_llm_override` on the silent path. The carve-out shape is
/// correct at the function level (this test verifies that); the call-site
/// wiring is the residual gap. Tracked in the follow-up ticket cited in the
/// mika#1217 PR description.
#[test]
fn test_resolve_skill_llm_override_silent_callback_and_deferred_dispatch_carve_out() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("moonshotai")
        .model_name("kimi-k2.5")
        .build();
    let self_dev =
        make_skill_entry_with_db_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));

    // Shape A — SilentTrigger::Callback: matched set wraps callback_safe_skills
    // with MatchReason::AlwaysOn. The carve-out must qualify the entry.
    let matched_callback = vec![MatchedSkill {
        entry: &self_dev,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_callback = matched_callback.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        qualifies_callback,
        "Callback turn: AlwaysOn skill with from_db_override=true must qualify for override"
    );
    // Resolution returns None because Settings is None (cannot construct provider),
    // but the early-exit at overrides.is_empty() is NOT taken — proving the
    // carve-out lets the entry through.
    let _ = resolve_skill_llm_override(&matched_callback, None, &mock, false);

    // Shape B — SilentTrigger::DeferredDispatch: identical matched-skill
    // construction to Callback. Same carve-out behavior expected.
    let matched_deferred = vec![MatchedSkill {
        entry: &self_dev,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_deferred = matched_deferred.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        qualifies_deferred,
        "DeferredDispatch turn: AlwaysOn skill with from_db_override=true must qualify for override"
    );
    let _ = resolve_skill_llm_override(&matched_deferred, None, &mock, false);

    // Negative control — same skill without from_db_override (developer-time
    // skill.toml [llm] source) must NOT qualify. #463 protection holds for
    // both Callback and DeferredDispatch shapes.
    let self_dev_dev_time =
        make_skill_entry_with_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched_dev_time = [MatchedSkill {
        entry: &self_dev_dev_time,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_dev_time = matched_dev_time.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        !qualifies_dev_time,
        "AlwaysOn skill without from_db_override (skill.toml [llm]) must NOT qualify (#463)"
    );
}

// -- mika#1011: intent guard trigger prefix tests --

#[test]
fn test_callback_trigger_excludes_deferred_dispatch() {
    // Regular callback should match
    assert!(callback_trigger_active(
        "[callback: long_running:run_claude_pilot]"
    ));
    // Deferred dispatch should NOT match callback_trigger_active
    assert!(!callback_trigger_active(
        "[callback:deferred-dispatch] [parent: task-123]"
    ));
}

/// mika#2355 AC2 / AC9 — the #870 guard no longer arms on a build
/// callback, and still arms on every other `long_running` callback.
///
/// `callback_trigger_active` is the ONE predicate both sites read — the
/// `INTENT_GUARDS` registry entry (non-empty text) and the inline
/// empty-text mirror — so asserting it here asserts both; the eval test
/// `test_qa_build_callback_verdict_2355` drives the production silent path
/// through each site end to end.
#[test]
fn mika2355_callback_terminal_action_is_carved_out_of_build_callbacks_only() {
    assert!(
        !callback_trigger_active("[callback: long_running:build_mika]"),
        "a build callback owns no self_dev parent to mark terminal"
    );
    // The milestone suffix must not re-arm it either.
    assert!(!callback_trigger_active(
        "[callback: long_running:build_mika] [milestone-parent: abc]"
    ));
    // The five other long_running tools keep the #870 contract verbatim.
    for tool in [
        "run_claude_pilot",
        "run_claude_pilot_groom",
        "deploy_mika",
        "address_pr_comments",
        "resolve_pr_conflicts",
    ] {
        assert!(
            callback_trigger_active(&format!("[callback: long_running:{tool}]")),
            "{tool}: the #870 guard must still arm"
        );
    }
    // Registry wiring: the entry's trigger IS this predicate, not a copy.
    let entry = INTENT_GUARDS
        .iter()
        .find(|g| g.label == CALLBACK_TERMINAL_ACTION_LABEL)
        .expect("the #870 entry exists");
    assert!(!(entry.trigger)("[callback: long_running:build_mika]"));
    assert!((entry.trigger)("[callback: long_running:run_claude_pilot]"));
}

/// mika#2355 AC3 / AC9 — the framing prescribes the self_dev terminal
/// contract to every callback EXCEPT a build callback, which is told what
/// its own contract is (a posted `run_gh pr review`).
#[test]
fn mika2355_build_callback_framing_does_not_prescribe_the_self_dev_contract() {
    let build = build_callback_trigger_context(
        crate::qa_build_callback::BUILD_CALLBACK_LABEL,
        "task-001",
        None,
        "Build succeeded",
        false,
    );
    assert!(
        !build.contains("This turn MUST end with both of the following"),
        "build callback framing still prescribes update_task_status + send_message"
    );
    assert!(
        !build.contains("mark the parent self_dev task terminal"),
        "build callback framing still names a self_dev parent"
    );
    assert!(build.contains("`run_gh` call with `pr review`"));
    assert!(build.contains("`VERDICT:`"));
    // The grounding and skill-delegation lines are common to all flows.
    assert!(build.contains("NEVER extrapolate to downstream states"));
    assert!(build.contains("Follow the workflow defined by your active skills"));

    for tool in [
        "run_claude_pilot",
        "run_claude_pilot_groom",
        "deploy_mika",
        "address_pr_comments",
        "resolve_pr_conflicts",
    ] {
        let ctx = build_callback_trigger_context(
            &format!("long_running:{tool}"),
            "task-001",
            None,
            "Result text",
            false,
        );
        assert!(
            ctx.contains("This turn MUST end with both of the following"),
            "{tool}: the self_dev terminal contract must be unchanged"
        );
        assert!(
            ctx.contains("1. update_task_status") && ctx.contains("2. send_message"),
            "{tool}: both terminal actions must still be named"
        );
        assert!(
            !ctx.contains("This is a BUILD callback"),
            "{tool}: must not receive the build-callback contract"
        );
    }
}

/// mika#2355 — the failure-verification instruction (#716) is orthogonal
/// to the flow: a FAILED build callback still gets it.
#[test]
fn mika2355_failed_build_callback_keeps_the_verification_instruction() {
    let ctx = build_callback_trigger_context(
        crate::qa_build_callback::BUILD_CALLBACK_LABEL,
        "task-001",
        None,
        "Build failed",
        true,
    );
    assert!(ctx.contains("This callback reported a FAILURE"));
    assert!(ctx.contains("This is a BUILD callback"));
}

#[test]
fn test_deferred_dispatch_trigger_matches_correctly() {
    assert!(deferred_dispatch_trigger(
        "[callback:deferred-dispatch] [parent: task-123]"
    ));
    assert!(!deferred_dispatch_trigger(
        "[callback: long_running:run_claude_pilot]"
    ));
    assert!(!deferred_dispatch_trigger("[heartbeat trigger]"));
}

#[test]
fn test_deferred_dispatch_satisfied_requires_run_claude_pilot() {
    // No tools → not satisfied
    assert!(!deferred_dispatch_satisfied(&[]));

    // send_message only → not satisfied
    assert!(!deferred_dispatch_satisfied(&[ToolCallSummary {
        name: "send_message".to_string(),
        input_summary: "".to_string(),
        output_summary: "".to_string(),
        success: true,
        non_zero_exit: false,
        step: 0,
    }]));

    // run_claude_pilot → satisfied (even if failed)
    assert!(deferred_dispatch_satisfied(&[ToolCallSummary {
        name: "run_claude_pilot".to_string(),
        input_summary: "".to_string(),
        output_summary: "".to_string(),
        success: false,
        non_zero_exit: false,
        step: 0,
    }]));
}

// mika#1173: regression tests asserting intent-guard satisfied predicates
// accept both run_claude_pilot AND run_claude_pilot_groom. The structural
// revert split grooming onto its own tool; without these guards updated,
// the auto-groom path on ready-label/milestone-cascade/deferred-replay
// would all fail at the EndTurn guard layer.
// (Reuses the `make_summary(name, output, success)` helper defined later in
// this test module; passing "" for output as these guards key on name+success.)

#[test]
fn test_ready_label_dispatch_satisfied_accepts_both_tools() {
    assert!(!ready_label_dispatch_satisfied(&[]));
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot",
        "",
        true
    )]));
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        true
    )]));
    // Failed attempts still count as "attempted" per the guard's contract.
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        false
    )]));
    assert!(!ready_label_dispatch_satisfied(&[make_summary(
        "send_message",
        "",
        true
    )]));
}

#[test]
fn test_deferred_dispatch_satisfied_accepts_groom_tool() {
    assert!(deferred_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        true
    )]));
    assert!(!deferred_dispatch_satisfied(&[make_summary(
        "update_task_status",
        "",
        true
    )]));
}

#[test]
fn test_callback_milestone_advance_satisfied_path_a_accepts_groom_tool() {
    let parent_id = "abc-123";
    // Path A: groom-tool advance.
    assert!(callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("run_claude_pilot_groom", "", true)]
    ));
    // Path A: implement-tool advance still works.
    assert!(callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("run_claude_pilot", "", true)]
    ));
    // Neither path → not satisfied.
    assert!(!callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("send_message", "", true)]
    ));
}

#[test]
fn test_webhook_no_unauthorized_dispatch_satisfied_rejects_both_tools() {
    // No tools called → satisfied (no unauthorized dispatch).
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[]));
    // Successful run_claude_pilot → unauthorized → NOT satisfied.
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&[
        make_summary("run_claude_pilot", "", true)
    ]));
    // Successful run_claude_pilot_groom → also unauthorized → NOT satisfied.
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&[
        make_summary("run_claude_pilot_groom", "", true)
    ]));
    // Failed dispatch attempts don't count (engine guards block them upstream).
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        false
    )]));
}

// -- detect_text_based_tool_call tests --

#[test]
fn test_detect_text_based_tool_call_function_tag() {
    assert!(detect_text_based_tool_call(
        "<function=search_memory>{\"query\":\"test\"}</function>"
    ));
}

#[test]
fn test_detect_text_based_tool_call_tool_call_wrapper() {
    assert!(detect_text_based_tool_call(
        "<tool_call><function=search>{}</function></tool_call>"
    ));
}

#[test]
fn test_detect_text_based_tool_call_plain_text() {
    assert!(!detect_text_based_tool_call(
        "I found some information about your meetings."
    ));
}

#[test]
fn test_detect_text_based_tool_call_empty() {
    assert!(!detect_text_based_tool_call(""));
}

#[test]
fn test_detect_text_based_tool_call_partial_tag() {
    // Only opening tag without closing — should NOT trigger.
    assert!(!detect_text_based_tool_call(
        "Use <function=search_memory to find things"
    ));
}

// -- detect_prose_style_tool_call tests (#569) --

/// Helper: build a tool name set from a slice of names.
fn tool_set(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_detect_prose_tool_call_known_tool() {
    let tools = tool_set(&["check_work_item"]);
    assert_eq!(
        detect_prose_style_tool_call(
            r#"check_work_item({"task_id": "48cbb025-6d8e-430f-a957-9ce2e32800bb"})"#,
            &tools,
        ),
        Some("check_work_item".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_whitespace_between_parens_and_brace() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"search_memory( {"query": "test"} )"#, &tools,),
        Some("search_memory".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_multiline_json() {
    let tools = tool_set(&["store_fact"]);
    let text = "store_fact(\n{\"key\": \"val\",\n\"key2\": \"val2\"}\n)";
    assert_eq!(
        detect_prose_style_tool_call(text, &tools),
        Some("store_fact".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_unknown_identifier() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"my_function({"key": "val"})"#, &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_empty_text() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(detect_prose_style_tool_call("", &tools), None);
}

#[test]
fn test_detect_prose_tool_call_parens_but_no_tool_pattern() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call("I found some information (see details).", &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_tool_name_without_invocation() {
    let tools = tool_set(&["check_work_item"]);
    assert_eq!(
        detect_prose_style_tool_call("Use check_work_item to verify the task status.", &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_generic_function_not_in_toolset() {
    let tools = tool_set(&["search_memory"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"Run my_func({"x": 1}) to test"#, &tools,),
        None,
    );
}

#[test]
fn test_detect_prose_tool_call_returns_first_match() {
    let tools = tool_set(&["search_memory", "store_fact"]);
    let text = r#"search_memory({"query": "a"}) and store_fact({"text": "b"})"#;
    assert_eq!(
        detect_prose_style_tool_call(text, &tools),
        Some("search_memory".to_string()),
    );
}

#[test]
fn test_detect_prose_tool_call_underscores_and_digits() {
    let tools = tool_set(&["tool_v2"]);
    assert_eq!(
        detect_prose_style_tool_call(r#"tool_v2({"a": 1})"#, &tools),
        Some("tool_v2".to_string()),
    );
}

// -- detect_completion_claim tests (#483) --

#[test]
fn test_detect_completion_claim_merged() {
    assert_eq!(
        detect_completion_claim("PR merged successfully"),
        Some("merged")
    );
}

#[test]
fn test_detect_completion_claim_completed() {
    assert_eq!(detect_completion_claim("Task completed"), Some("completed"));
}

#[test]
fn test_detect_completion_claim_complete() {
    assert_eq!(
        detect_completion_claim("The migration is complete"),
        Some("complete")
    );
}

#[test]
fn test_detect_completion_claim_deployed() {
    assert_eq!(
        detect_completion_claim("Successfully deployed to production"),
        Some("deployed")
    );
}

#[test]
fn test_detect_completion_claim_shipped() {
    assert_eq!(
        detect_completion_claim("Feature shipped in v2.1"),
        Some("shipped")
    );
}

#[test]
fn test_detect_completion_claim_case_insensitive() {
    assert_eq!(detect_completion_claim("MERGED the PR"), Some("MERGED"));
    assert_eq!(
        detect_completion_claim("Successfully Deployed"),
        Some("Deployed")
    );
}

#[test]
fn test_detect_completion_claim_no_match_on_done() {
    // "done" intentionally excluded — too many false positives
    assert!(detect_completion_claim("I'm done analyzing").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_built() {
    // "built" intentionally excluded — too many false positives
    assert!(detect_completion_claim("I built a query for you").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_finished() {
    assert!(detect_completion_claim("I finished the analysis").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_plain_text() {
    assert!(detect_completion_claim("Here is the analysis result").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_substring() {
    // "unmerged" should NOT match due to word boundary
    assert!(detect_completion_claim("the unmerged changes").is_none());
}

#[test]
fn test_detect_completion_claim_no_match_on_empty() {
    assert!(detect_completion_claim("").is_none());
}

#[test]
fn test_detect_completion_claim_word_boundary_redeployed() {
    // "redeployed" contains "deployed" but word boundary should still match
    // because "redeployed" has "deployed" at a word boundary after the prefix
    // Actually regex \b matches between "re" and "deployed"? No — \bdeployed\b
    // requires a word boundary before "deployed". In "redeployed", there's no
    // boundary between "re" and "deployed", so it should NOT match.
    assert!(detect_completion_claim("the service was redeployed").is_none());
}

#[test]
fn test_detect_completion_claim_in_sentence() {
    assert_eq!(
        detect_completion_claim("I've merged the PR and synced main. Everything looks good."),
        Some("merged")
    );
}

// -- detect_milestone_close_claim_without_patch tests (#797, #1207) --

/// Helper: build a ToolCallSummary for run_gh with the given input.
fn run_gh_summary(input: &str) -> ToolCallSummary {
    ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: input.to_string(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }
}

#[test]
fn test_detect_milestone_close_claim_with_patch_passes() {
    // First-person claim AND matching PATCH → returns None (no violation).
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries,)
            .is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_without_patch_caught() {
    // First-person claim AND no PATCH argv → returns the matched keyword.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I closed milestone#17, tasks reconciled, memory updated",
        &summaries,
    );
    assert!(result.is_some());
    assert!(result.unwrap().contains("closed"));
}

#[test]
fn test_detect_milestone_close_claim_case_insensitive() {
    // "I CLOSED MILESTONE" caught regardless of case.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I CLOSED MILESTONE #17 successfully",
        &summaries,
    );
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_no_match_on_unrelated_close() {
    // "PR closed" alone does NOT trigger — no first-person + milestone.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch("PR closed successfully", &summaries).is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_readback_alone_not_sufficient() {
    // Only readback argv (api ... --jq .state), no PATCH → still triggers.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","/repos/senara-solutions/mika/milestones/17","--jq",".state"]}"#,
    )];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_no_match_on_empty_text() {
    let summaries = vec![];
    assert!(detect_milestone_close_claim_without_patch("", &summaries).is_none());
}

#[test]
fn test_detect_milestone_close_claim_third_person_no_longer_matches() {
    // #1207: third-person planning prose no longer triggers the guard.
    // This replaces the old `_future_tense_overmatches_intentionally` test.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch(
            "The milestone is ready to close now",
            &summaries,
        )
        .is_none()
    );
}

#[test]
fn test_detect_milestone_close_claim_planning_prose_no_match() {
    // #1207: the incident text — mika-arch reviewing a brief about
    // milestone workflows. Third-person planning shape must not fire.
    let summaries = vec![];
    assert!(detect_milestone_close_claim_without_patch(
            "the plan proposes mika-dev call gh api PATCH /repos/owner/repo/milestones/789 to close the milestone",
            &summaries,
        )
        .is_none());
}

#[test]
fn test_detect_milestone_close_claim_planning_prose_no_number_no_match() {
    // #1207 C3b: third-person without API path or number.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch(
            "the plan proposes mika-dev close the milestone after merge",
            &summaries,
        )
        .is_none()
    );
}

#[test]
fn test_dual_trigger_completion_and_milestone_close_both_detect() {
    // When text contains both "completed" and first-person milestone claim,
    // both detection functions fire — but the post-condition chain's
    // sequential ordering means only #4 (completion-claim) fires first.
    let text = "I completed milestone#17 and closed it";
    let summaries: Vec<ToolCallSummary> = vec![];

    // Completion-claim detector: fires on "completed".
    assert!(detect_completion_claim(text).is_some());
    // Milestone-close detector: fires on first-person "completed...milestone".
    assert!(detect_milestone_close_claim_without_patch(text, &summaries).is_some());
}

#[test]
fn test_milestone_close_fires_after_completion_claim_satisfied() {
    // Agent called update_task_status (satisfies guard #4) but no PATCH.
    let text = "I completed milestone#17 and closed it";
    let summaries: Vec<ToolCallSummary> = vec![ToolCallSummary {
        step: 1,
        name: "update_task_status".to_string(),
        input_summary: r#"{"task_id":"abc","status":"completed"}"#.to_string(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];

    assert!(detect_completion_claim(text).is_some());
    // Milestone-close: still fires because no PATCH call in summaries.
    assert!(detect_milestone_close_claim_without_patch(text, &summaries).is_some());
}

#[test]
fn test_detect_milestone_close_claim_ive_form() {
    // "I've closed" should match.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I've closed milestone#14 on GitHub",
        &summaries,
    );
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_we_form() {
    // "We completed" should match.
    let summaries = vec![];
    let result =
        detect_milestone_close_claim_without_patch("We completed milestone#15 today", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_weve_form() {
    // "We've closed out" should match.
    let summaries = vec![];
    let result =
        detect_milestone_close_claim_without_patch("We've closed out milestone#20", &summaries);
    assert!(result.is_some());
}

#[test]
fn test_detect_milestone_close_claim_closed_out_form() {
    // "I closed out" should match.
    let summaries = vec![];
    let result = detect_milestone_close_claim_without_patch(
        "I closed out the milestone for mika#789",
        &summaries,
    );
    assert!(result.is_some());
}

// -- extract_claimed_milestone_number tests (#1207) --

#[test]
fn test_extract_claimed_milestone_number_with_hash() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone#14"),
        Some(14)
    );
}

#[test]
fn test_extract_claimed_milestone_number_without_hash() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone 14"),
        Some(14)
    );
}

#[test]
fn test_extract_claimed_milestone_number_with_space_hash() {
    assert_eq!(extract_claimed_milestone_number("milestone # 14"), Some(14));
}

#[test]
fn test_extract_claimed_milestone_number_no_number() {
    assert_eq!(
        extract_claimed_milestone_number("I closed the milestone"),
        None
    );
}

#[test]
fn test_extract_claimed_milestone_number_multiple_takes_first() {
    assert_eq!(
        extract_claimed_milestone_number("I closed milestone#14 not milestone#789"),
        Some(14)
    );
}

// -- PATCH set extraction tests (#1207) --

#[test]
fn test_patch_set_single_patch() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#,
    )];
    // Claim matches the PATCH number → suppressed.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17", &summaries,).is_none()
    );
}

#[test]
fn test_patch_set_multiple_patches() {
    let summaries = vec![
        run_gh_summary(
            r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/14","-f","state=closed"]}"#,
        ),
        run_gh_summary(
            r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/789","-f","state=closed"]}"#,
        ),
    ];
    // Claimed #14, #14 is in the PATCH set → suppressed.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_none()
    );
}

#[test]
fn test_patch_set_no_patches() {
    let summaries = vec![];
    // No patches, first-person claim → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_patch_set_malformed_url_skipped() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/milestones/notanumber","-f","state=closed"]}"#,
    )];
    // Malformed URL doesn't contribute to PATCH set → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_patch_set_non_milestones_path_skipped() {
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/issues/14","-f","state=closed"]}"#,
    )];
    // PATCH to issues, not milestones → fires.
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

// -- Cross-milestone discrimination tests (#1207) --

#[test]
fn test_cross_milestone_claim_different_number_fires() {
    // AC5 from #1207: claim milestone#14 but PATCH milestone#789 → fires.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/789","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#14", &summaries,).is_some()
    );
}

#[test]
fn test_no_number_claim_with_patch_suppressed() {
    // Claim without number + any PATCH → suppressed (presence/absence fallback).
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed the milestone on GitHub", &summaries,)
            .is_none()
    );
}

#[test]
fn test_no_number_claim_without_patch_fires() {
    // Claim without number + no PATCH → fires.
    let summaries = vec![];
    assert!(
        detect_milestone_close_claim_without_patch("I closed the milestone on GitHub", &summaries,)
            .is_some()
    );
}

// -- parse_run_gh_milestone_close_argv unit tests (#1182) --

#[test]
fn test_parse_run_gh_milestone_close_argv_valid() {
    let input =
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/42","-f","state=closed"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), Some(42));
}

#[test]
fn test_parse_run_gh_milestone_close_argv_not_api() {
    let input = r#"{"command":["pr","comment","--body","text"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_no_state_closed() {
    // PATCH without state=closed should not match.
    let input =
        r#"{"command":["api","-X","PATCH","/repos/o/r/milestones/17","-f","title=new name"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_truncated_json() {
    // Truncated JSON should return None (graceful fallback).
    let input = r#"{"command":["api","-X","PATCH","/repos/o/r/milest"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), None);
}

#[test]
fn test_parse_run_gh_milestone_close_argv_long_method_flag() {
    // "--method" is the long form of "-X" in gh api.
    let input =
        r#"{"command":["api","--method","PATCH","/repos/o/r/milestones/17","-f","state=closed"]}"#;
    assert_eq!(parse_run_gh_milestone_close_argv(input), Some(17));
}

// -- milestone-close guard hardening integration tests (#1182) --

#[test]
fn test_milestone_close_guard_substring_spoof_rejected() {
    // A pr comment whose body contains all four substrings should NOT
    // satisfy the guard — the PATCH is in the body text, not an actual
    // api PATCH call.
    let spoofed = run_gh_summary(
        r#"{"command":["pr","comment","--body","closed via PATCH /repos/senara-solutions/mika/milestones/17 state=closed"]}"#,
    );
    let summaries = vec![spoofed];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // Guard should fire — the spoof should not satisfy it.
    assert!(
        result.is_some(),
        "substring spoof should not satisfy the guard"
    );
}

#[test]
fn test_milestone_close_guard_cross_milestone_leakage() {
    // PATCH for milestone#17 should NOT satisfy a claim about milestone#18.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","-X","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#18 on GitHub", &summaries);
    assert!(
        result.is_some(),
        "PATCH for #17 should not satisfy claim about #18"
    );
}

#[test]
fn test_milestone_close_guard_truncated_input_still_resolves() {
    // Build an argv where state=closed is beyond byte 200 (truncation boundary).
    // The structured parse fails on truncated JSON, but the path and "PATCH"
    // appear before byte 200, so the substring fallback extracts the number.
    let long_org = "a".repeat(150); // Push state=closed past byte 200
    let input = format!(
        r#"{{"command":["api","-X","PATCH","/repos/{}/mika/milestones/17","-f","state=closed"]}}"#,
        long_org
    );
    // Truncate to crate::planning::policy::INPUT_SUMMARY_MAX to simulate what ToolCallSummary does.
    let truncated =
        crate::tool_execution::truncate_summary(&input, crate::planning::policy::INPUT_SUMMARY_MAX);
    let summaries = vec![run_gh_summary(&truncated)];

    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // If state=closed was truncated, substring fallback also fails →
    // guard correctly fires (over-fire is the safe direction per the ticket).
    // The key invariant: the guard does NOT stall — it either suppresses
    // (if substring fallback finds a match) or fires (if not).
    // With 150-char org name, state=closed is truncated away, so the guard fires.
    assert!(
        result.is_some(),
        "truncated state=closed should cause guard to fire (safe direction)"
    );
}

#[test]
fn test_milestone_close_guard_long_method_flag() {
    // "--method" is the long form of "-X" in gh api.
    let summaries = vec![run_gh_summary(
        r#"{"command":["api","--method","PATCH","/repos/senara-solutions/mika/milestones/17","-f","state=closed"]}"#,
    )];
    assert!(
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries,)
            .is_none(),
        "--method PATCH should satisfy the guard"
    );
}

#[test]
fn test_milestone_close_guard_non_json_fallback() {
    // If input_summary is not valid JSON (e.g., pre-existing format or
    // corruption), the substring fallback should still work.
    let summaries = vec![run_gh_summary(
        r#"api -X PATCH /repos/senara-solutions/mika/milestones/17 -f state=closed "api" "PATCH""#,
    )];
    // This is a non-JSON string that happens to contain the substring markers.
    // The structured parse fails; the substring fallback should try to extract.
    let result =
        detect_milestone_close_claim_without_patch("I closed milestone#17 on GitHub", &summaries);
    // Substring fallback finds "api", "PATCH", and "state=closed" as substrings,
    // plus the milestone path regex matches. Should suppress the guard.
    assert!(
        result.is_none(),
        "substring fallback should work for non-JSON input"
    );
}

// -- Persistence evaluation guard detection tests (#648) --

#[test]
fn test_detect_informational_input_fyi() {
    let result = detect_informational_input("FYI the deploy succeeded");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "FYI");
}

#[test]
fn test_detect_informational_input_heads_up() {
    let result = detect_informational_input("Heads up — the CI pipeline is red");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "Heads up");
}

#[test]
fn test_detect_informational_input_diagnostic() {
    let result = detect_informational_input("Running a diagnostic on the memory system");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "diagnostic");
}

#[test]
fn test_detect_informational_input_correction() {
    let result = detect_informational_input("actually, the timeout is 30s not 60s");
    assert!(result.is_some());
    // trim_start() ensures no leading whitespace from the (?:^|\s) anchor
    assert_eq!(result.unwrap(), "actually,");
}

#[test]
fn test_detect_informational_input_no_match() {
    assert!(detect_informational_input("Can you fix the bug in the parser?").is_none());
}

#[test]
fn test_detect_informational_input_empty() {
    assert!(detect_informational_input("").is_none());
}

#[test]
fn test_detect_persistable_output_root_cause() {
    let result =
        detect_persistable_output("The root cause was a connection timeout in the retry loop");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "root cause");
}

#[test]
fn test_detect_persistable_output_confirms() {
    let result =
        detect_persistable_output("This confirms the diagnosis — the issue was in the parser");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "This confirms");
}

#[test]
fn test_detect_persistable_output_validated() {
    let result = detect_persistable_output("I validated that the fix works correctly");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "validated that");
}

#[test]
fn test_detect_persistable_output_lesson_learned() {
    let result = detect_persistable_output("Key lesson learned: always check the return code");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "lesson learned");
}

#[test]
fn test_detect_persistable_output_no_match() {
    assert!(detect_persistable_output("Here's the code change you requested").is_none());
}

#[test]
fn test_detect_persistable_output_future_tense_no_match() {
    // "I'll verify" is future-tense — should not trigger
    assert!(detect_persistable_output("I'll verify the fix later").is_none());
}

#[test]
fn test_detect_persistable_output_empty() {
    assert!(detect_persistable_output("").is_none());
}

#[test]
fn test_detect_informational_input_incident_report() {
    let result = detect_informational_input("incident report: prod database was down for 5 min");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), "incident report");
}

// -- is_terminal_tool_error tests --

#[test]
fn test_terminal_error_github_self_approval() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nGraphQL: Can not approve your own pull request"
    ));
}

#[test]
fn test_terminal_error_github_self_review() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nYou can't review your own pull request"
    ));
}

#[test]
fn test_terminal_error_http_404() {
    assert!(is_terminal_tool_error("HTTP 404: Not Found"));
}

#[test]
fn test_terminal_error_http_403() {
    assert!(is_terminal_tool_error("HTTP 403: Forbidden"));
}

#[test]
fn test_terminal_error_http_401() {
    assert!(is_terminal_tool_error("HTTP 401: Unauthorized"));
}

#[test]
fn test_terminal_error_insufficient_permissions() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\ninsufficient permissions for this resource"
    ));
}

#[test]
fn test_terminal_error_resource_not_accessible() {
    assert!(is_terminal_tool_error(
        "Resource not accessible by integration"
    ));
}

#[test]
fn test_terminal_error_permission_denied() {
    assert!(is_terminal_tool_error("permission denied"));
}

#[test]
fn test_terminal_error_case_insensitive() {
    assert!(is_terminal_tool_error(
        "Exit code: 1\nGraphQL: CAN NOT APPROVE YOUR OWN pull request"
    ));
}

#[test]
fn test_retryable_error_http_429() {
    assert!(!is_terminal_tool_error("HTTP 429: rate limit exceeded"));
}

#[test]
fn test_retryable_error_rate_limit() {
    assert!(!is_terminal_tool_error(
        "API rate limit exceeded. Please retry after 60 seconds."
    ));
}

#[test]
fn test_retryable_error_http_500() {
    assert!(!is_terminal_tool_error("HTTP 500: Internal Server Error"));
}

#[test]
fn test_retryable_error_http_502() {
    assert!(!is_terminal_tool_error("HTTP 502: Bad Gateway"));
}

#[test]
fn test_retryable_error_http_503() {
    assert!(!is_terminal_tool_error("HTTP 503: Service Unavailable"));
}

#[test]
fn test_retryable_error_http_504() {
    assert!(!is_terminal_tool_error("HTTP 504: Gateway Timeout"));
}

#[test]
fn test_retryable_error_timeout() {
    assert!(!is_terminal_tool_error("request timed out after 30s"));
}

#[test]
fn test_retryable_error_connection_refused() {
    assert!(!is_terminal_tool_error("connection refused"));
}

#[test]
fn test_retryable_overrides_terminal() {
    // A 429 with "permission denied" text should still be retryable
    assert!(!is_terminal_tool_error(
        "HTTP 429: permission denied rate limit exceeded"
    ));
}

#[test]
fn test_not_found_bare_is_not_terminal() {
    // Bare "not found" should NOT be terminal — too broad, matches search results
    assert!(!is_terminal_tool_error("No matching records found"));
    assert!(!is_terminal_tool_error("file not found: /tmp/data"));
}

#[test]
fn test_forbidden_bare_is_not_terminal() {
    // Bare "forbidden" should NOT be terminal — too broad
    assert!(!is_terminal_tool_error(
        "some resources are forbidden by default"
    ));
}

#[test]
fn test_unauthorized_bare_is_not_terminal() {
    // Bare "unauthorized" should NOT be terminal — too broad
    assert!(!is_terminal_tool_error(
        "unauthorized access attempt logged"
    ));
}

#[test]
fn test_unknown_error_not_terminal() {
    assert!(!is_terminal_tool_error("some random error message"));
}

#[test]
fn test_empty_output_not_terminal() {
    assert!(!is_terminal_tool_error(""));
}

#[test]
fn test_successful_output_not_terminal() {
    assert!(!is_terminal_tool_error("PR approved successfully"));
}

// -- has_terminal_required_tool_failure tests --

fn make_summary(name: &str, output: &str, success: bool) -> ToolCallSummary {
    ToolCallSummary {
        step: 0,
        name: name.to_string(),
        input_summary: String::new(),
        output_summary: output.to_string(),
        success,
        non_zero_exit: !success,
    }
}

#[test]
fn test_terminal_required_tool_failure_detected() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "Exit code: 1\nGraphQL: Can not approve your own pull request",
        false,
    )];
    assert!(has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_non_required_tool_ignored() {
    let required: HashSet<String> = ["qa_pr_view"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "Exit code: 1\nGraphQL: Can not approve your own pull request",
        false,
    )];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_successful_tool_ignored() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "PR approved successfully", true)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_retryable_error_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary(
        "run_gh",
        "HTTP 429: rate limit exceeded",
        false,
    )];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_unknown_error_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "some random error", false)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_empty_summaries() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    assert!(!has_terminal_required_tool_failure(&required, &[]));
}

#[test]
fn test_terminal_failure_empty_output_not_terminal() {
    let required: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
    let summaries = vec![make_summary("run_gh", "", false)];
    assert!(!has_terminal_required_tool_failure(&required, &summaries));
}

#[test]
fn test_terminal_failure_multiple_summaries_one_terminal() {
    let required: HashSet<String> = ["run_gh", "qa_pr_view"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let summaries = vec![
        make_summary("qa_pr_view", "PR #42: Fix bug", true),
        make_summary("run_gh", "Exit code: 1\nHTTP 404: Not Found", false),
    ];
    assert!(has_terminal_required_tool_failure(&required, &summaries));
}

// -- has_successful_pr_review tests (#695) --

#[test]
fn pr_review_detected_when_run_gh_pr_review_succeeded() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","455","--approve","--body","VERDICT: pass"]}"#
            .to_string(),
        output_summary: "Review submitted".to_string(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_for_failed_review() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","455","--approve"]}"#.to_string(),
        output_summary: "Exit code: 1\nHTTP 422: Unprocessable Entity".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(!has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_for_pr_list() {
    let summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","list","--state","open"]}"#.to_string(),
        output_summary: "PR #1 Fix bug".to_string(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!has_successful_pr_review(&summaries));
}

#[test]
fn pr_review_not_detected_when_empty() {
    assert!(!has_successful_pr_review(&[]));
}

// -- required-tools gate skipped after PR review success (#821, Fix B) --

#[test]
fn required_tools_gate_skipped_after_pr_review_success() {
    // Simulate: successful `run_gh pr review` in tool summaries.
    // Required tools include a tool not called (e.g., qa_pr_view).
    // has_successful_pr_review should return true, which means the
    // required-tools gate should skip instead of re-prompting.
    let summaries = vec![
            ToolCallSummary {
                step: 1,
                name: "run_gh".to_string(),
                input_summary:
                    r#"{"command":["pr","review","https://github.com/org/repo/pull/42","--approve","--body","VERDICT: pass"]}"#
                        .to_string(),
                output_summary: "Review submitted".to_string(),
                success: true,
                non_zero_exit: false,
            },
        ];

    // This is the condition checked in the required-tools gate (Fix B).
    // When true, the gate skips re-prompting.
    assert!(
        has_successful_pr_review(&summaries),
        "should detect successful PR review in tool summaries"
    );

    // Verify it returns false for the negative case (failed review).
    let failed_summaries = vec![ToolCallSummary {
        step: 1,
        name: "run_gh".to_string(),
        input_summary: r#"{"command":["pr","review","42","--approve"]}"#.to_string(),
        output_summary: "Exit code: 1".to_string(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(
        !has_successful_pr_review(&failed_summaries),
        "failed review should not trigger early-accept"
    );
}

// -- detect_resume_intent tests --

#[test]
fn resume_intent_detected_for_milestone() {
    assert!(detect_resume_intent("resume mika milestone#8"));
    assert!(detect_resume_intent("Resume mika milestone#8"));
    assert!(detect_resume_intent("please resume mika milestone #8"));
}

#[test]
fn resume_intent_detected_for_project() {
    assert!(detect_resume_intent("continue mika project#3"));
    assert!(detect_resume_intent("Continue mika project #3"));
}

#[test]
fn resume_intent_not_detected_without_process_ref() {
    assert!(!detect_resume_intent("resume the task"));
    assert!(!detect_resume_intent("continue working on the feature"));
    assert!(!detect_resume_intent("resume"));
}

#[test]
fn resume_intent_not_detected_without_verb() {
    assert!(!detect_resume_intent("check mika milestone#8"));
    assert!(!detect_resume_intent("milestone#8 status"));
}

#[test]
fn resume_intent_not_detected_on_regular_messages() {
    assert!(!detect_resume_intent("How's the project going?"));
    assert!(!detect_resume_intent("Can you explain the architecture?"));
    assert!(!detect_resume_intent(""));
}

// -- IntentScope tests (mika#3002) --

/// Un plan de groom ordinaire : il parle de reprise parce que c'est son sujet, et il
/// cite un jalon. Les deux conjoints de `detect_resume_intent` sont là, et aucune
/// intention de reprise n'est adressée à l'agent.
///
/// Les deux lignes qui portent les conjoints sont celles **mesurées** sur le plan de
/// mika#2995 (`tests/fixtures/review_anchor_unparsed_retry_2996/plan-mika2995-61a3fb572.md`,
/// l. 479 et 506), le brief de l'incident `3a066c59` : le verbe vient du français « le
/// prompt servi *continue* de prescrire », que `\b(resume|continue)\b` attrape mot pour
/// mot, et le processus d'un « milestone #33 » **cité**. « reprendre le vol suivant »,
/// que nomme AC2, est là aussi mais ne porte aucun des deux — c'est le vocabulaire du
/// défaut, pas son déclencheur.
const PLAN_DE_GROOM: &str = "## 4. Ordre de vol\n\nLe premier vol livre le détecteur ; \
         il faudra reprendre le vol suivant une fois la sonde relevée.\n\nLe défaut survit \
         au correctif : le prompt servi continue de prescrire l'ancienne forme.\n\nLié : \
         milestone #33, dont ce ticket est l'enfant.\n";

/// Le plan sous l'enveloppe de mika#2996, précédé d'une vraie instruction à l'agent.
fn brief_enveloppe(tete: &str) -> String {
    format!(
        "{tete}{}\n{PLAN_DE_GROOM}{}\n",
        review_anchor::RETRY_PLAN_BEGIN,
        review_anchor::RETRY_PLAN_END
    )
}

/// Le contrôle qui fait tenir tous les autres : ce plan déclenche bien la garde quand
/// le message entier est lu. Sans lui, les tests de stand-down passeraient aussi sur un
/// plan que le détecteur n'a jamais vu.
#[test]
fn mika3002_le_plan_de_groom_declenche_sur_le_message_entier() {
    assert!(
        IntentScope::WholeMessage.triggered(detect_resume_intent, PLAN_DE_GROOM, true),
        "la fixture ne porte plus les deux conjoints : les cas de stand-down ne \
             mesureraient rien"
    );
}

/// AC1 — sur un tour de revue, un brief sans enveloppe EST le plan : rien n'y est
/// adressé à l'agent, donc la garde ne s'arme pas. C'est la forme de l'incident
/// `3a066c59`.
#[test]
fn mika3002_sur_un_tour_de_revue_le_plan_seul_ne_declenche_pas() {
    assert!(!IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, PLAN_DE_GROOM, true));
}

/// Hors tour de revue, le rétrécissement ne s'applique pas : même entrée, même
/// message, comportement d'avant mika#3002. C'est ce qui borne le stand-down à sa
/// population.
#[test]
fn mika3002_hors_tour_de_revue_le_comportement_est_inchange() {
    assert!(IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, PLAN_DE_GROOM, false));
}

/// AC2 — une vraie instruction de reprise adressée à l'agent vit HORS du plan, donc
/// dans la région lue : la garde s'arme toujours.
#[test]
fn mika3002_une_instruction_hors_du_plan_declenche_toujours() {
    let brief = brief_enveloppe("resume mika milestone#8\n\n");
    assert!(IntentScope::OutsideReviewedPlan.triggered(detect_resume_intent, &brief, true));
}

/// `stood_down` ne dit vrai que quand le rétrécissement a réellement changé l'issue —
/// c'est le contrat de la sonde de contrôle positif, pas un synonyme de « tour de
/// revue ».
#[test]
fn mika3002_stood_down_ne_vaut_que_quand_le_retrecissement_change_lissue() {
    // Le cas qu'AC3 compte : le message entier déclenchait, la région lue non.
    assert!(IntentScope::OutsideReviewedPlan.stood_down(detect_resume_intent, PLAN_DE_GROOM, true));
    // Une instruction hors plan : la garde s'arme, donc aucun stand-down.
    let brief = brief_enveloppe("resume mika milestone#8\n\n");
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(detect_resume_intent, &brief, true));
    // Hors tour de revue : rien n'a été rétréci.
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(
        detect_resume_intent,
        PLAN_DE_GROOM,
        false
    ));
    // Un message sans intention de reprise : il n'y avait rien à désarmer.
    assert!(!IntentScope::OutsideReviewedPlan.stood_down(
        detect_resume_intent,
        "Can you explain the architecture?",
        true
    ));
    // Une entrée à portée entière ne se désarme jamais, même sur un tour de revue.
    assert!(!IntentScope::WholeMessage.stood_down(detect_resume_intent, PLAN_DE_GROOM, true));
}

/// `resume_reconcile` est la seule entrée rétrécie. Élargir le stand-down au reste du
/// registre serait un changement non mesuré déguisé en cohérence : aucune autre entrée
/// n'a de prémisse fausse sur un tour de revue, toutes exigent un préfixe `[GitHub]` ou
/// `[callback: …]` qu'un brief de plan ne porte pas.
#[test]
fn mika3002_resume_reconcile_est_la_seule_entree_retrecie() {
    let retrecies: Vec<&str> = INTENT_GUARDS
        .iter()
        .filter(|g| matches!(g.scope, IntentScope::OutsideReviewedPlan))
        .map(|g| g.label)
        .collect();
    assert_eq!(retrecies, vec!["resume_reconcile"]);
}

// -- resume_reconcile_satisfied tests --

#[test]
fn resume_satisfied_with_successful_list_tasks() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "list_tasks".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_satisfied_with_successful_check_task() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "check_task".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_with_failed_list_tasks() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "list_tasks".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: false,
    }];
    assert!(!resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_with_unrelated_tool() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "send_message".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!resume_reconcile_satisfied(&summaries));
}

#[test]
fn resume_not_satisfied_when_empty() {
    assert!(!resume_reconcile_satisfied(&[]));
}

// -- ready_label_dispatch trigger tests (#846) --

#[test]
fn ready_label_trigger_matches_canonical_marker() {
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#999"
    ));
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — title with — em-dashes"
    ));
    // Body containing additional text after the marker still matches.
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#42\n\nIssue body follows."
    ));
}

#[test]
fn ready_label_trigger_rejects_other_labels() {
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled bug on mika#999"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled p1-important on mika#999"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled enhancement on mika#999"
    ));
}

#[test]
fn ready_label_trigger_rejects_other_event_types() {
    // Other GitHub events that share the `[GitHub]` prefix must not trigger.
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] PR comment on mika#999 by samidarko"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue comment on mika#999 by samidarko"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Check suite failure on branch fix/foo"
    ));
}

#[test]
fn ready_label_trigger_rejects_direct_prompts() {
    // Direct `mika ask` prompts with no source-prefix must not trigger.
    assert!(!ready_label_dispatch_trigger("implement mika issue#999"));
    assert!(!ready_label_dispatch_trigger(
        "ready label on mika#999 please"
    ));
    assert!(!ready_label_dispatch_trigger(""));
}

// -- ready_label_dispatch_satisfied tests --

#[test]
fn ready_label_satisfied_when_run_claude_pilot_succeeded() {
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "create_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_only_run_gh_succeeded() {
    // Reproduces the #846 regression: label removal succeeded but
    // run_claude_pilot was never called.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_gh".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_satisfied_when_run_claude_pilot_attempted_terminally() {
    // Attempt with terminal failure (e.g., global_dispatch_active when a
    // concurrent dispatch is active) still satisfies the guard. Forcing
    // a retry on terminal failures would produce misleading "never called"
    // operator notifications.  The LLM handles the failure via send_message
    // per the prompt's Step 4 (#846 adversarial review).
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_empty() {
    assert!(!ready_label_dispatch_satisfied(&[]));
}

// -- #1089: send_message no longer satisfies the guard --
// Post-#996, all legitimate completion paths call run_claude_pilot
// (dispatch via dev-pilot, auto-groom via dev-groom). The send_message-only
// path was removed after fabricated check_task pre-flights exploited the
// over-broad OR-shape to short-circuit dispatch via NoChannel escalation.

#[test]
fn ready_label_not_satisfied_when_only_send_message_called() {
    // #1089 — send_message alone no longer satisfies the guard.
    // Previously this was the grooming-rejection path (#907); post-#996
    // auto-groom replaced it with run_claude_pilot(dev-groom).
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_satisfied_when_both_dispatch_and_notification() {
    // Both run_claude_pilot and send_message present (e.g., dispatch
    // succeeded then operator was notified). Satisfied because
    // run_claude_pilot is present.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_only_send_message_failed() {
    // #1089 — send_message (even failed) does not satisfy the guard.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "send_message".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: false,
    }];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_fabricated_check_task_then_send_message() {
    // #1089 — reproduces the exact fabrication pattern: agent calls
    // check_task (stale, fails), then send_message (fabricated escalation).
    // Guard must reject because run_claude_pilot was never called.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "check_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: false,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

// -- parse_ready_label_location tests (#846 operator notification) --

#[test]
fn parse_location_from_simple_marker() {
    let out = parse_ready_label_location("[GitHub] Issue labeled ready on mika#999");
    assert_eq!(out, Some("mika#999".to_string()));
}

#[test]
fn parse_location_from_marker_with_em_dash_title() {
    let out = parse_ready_label_location(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 \u{2014} title with \u{2014} more dashes",
    );
    assert_eq!(out, Some("senara-solutions/mika#1234".to_string()));
}

#[test]
fn parse_location_from_marker_with_newline_body() {
    let out = parse_ready_label_location(
        "[GitHub] Issue labeled ready on mika#42\n\nIssue body follows.",
    );
    assert_eq!(out, Some("mika#42".to_string()));
}

#[test]
fn parse_location_returns_none_for_other_markers() {
    assert!(parse_ready_label_location("[GitHub] Issue labeled bug on mika#999").is_none());
    assert!(parse_ready_label_location("implement mika issue#999").is_none());
    assert!(parse_ready_label_location("").is_none());
}

#[test]
fn parse_location_returns_none_for_empty_location() {
    // Marker present but no location text after it.
    assert!(parse_ready_label_location("[GitHub] Issue labeled ready on ").is_none());
    assert!(parse_ready_label_location("[GitHub] Issue labeled ready on \n").is_none());
}

#[test]
fn ready_label_guard_runs_before_webhook_zero_tools() {
    // Registry-order invariant: the more specific ready-label guard must
    // be evaluated before the generic webhook_zero_tools guard. Otherwise
    // a successful run_gh on the ready-label turn satisfies
    // webhook_zero_tools and the missing run_claude_pilot is never caught.
    let labels: Vec<&str> = INTENT_GUARDS.iter().map(|g| g.label).collect();
    let ready_idx = labels
        .iter()
        .position(|l| *l == "webhook_ready_label_dispatch")
        .expect("webhook_ready_label_dispatch must be registered");
    let zero_idx = labels
        .iter()
        .position(|l| *l == "webhook_zero_tools")
        .expect("webhook_zero_tools must be registered");
    assert!(
        ready_idx < zero_idx,
        "webhook_ready_label_dispatch (idx={ready_idx}) must precede \
             webhook_zero_tools (idx={zero_idx}) so the more specific trigger \
             fires first"
    );
}

// -- #1469 webhook_zero_tools_trigger prefix-narrowing tests --

#[test]
fn webhook_zero_tools_trigger_skips_check_suite_success() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] Check suite success on senara-solutions/mika (branch: main)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_pr_closed() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_discussion() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] discussion.created on senara-solutions/mika"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_check_suite_failure() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_ready_label() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_pr_review() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_non_github() {
    assert!(!webhook_zero_tools_trigger("[Slack] message"));
    assert!(!webhook_zero_tools_trigger(""));
}

// -- mika#2517 U3 — the fallthrough domain no longer arms this guard --

/// **V4** — the five shapes of the Webhook Fallthrough domain stand the
/// guard down; the three shapes outside it still arm it.
///
/// **This inverts two assertions mika#1469 shipped.** `fires_on_new_comment`
/// and `fires_on_non_ready_label` asserted that a comment and a non-`ready`
/// label re-prompt a turn with zero tool calls. On exactly that population
/// the guard's premise — *"webhook events require action"* — contradicts the
/// `self-dev` HARD GATE, which says the correct action is to acknowledge and
/// stop. The old assertions were pinning the conflict, so they are replaced
/// here rather than carried: keeping them would mean the suite asserts both
/// halves of a contradiction.
///
/// The positive half is what makes this non-vacuous. A trigger that returned
/// `false` on everything would satisfy the negative half alone — and would
/// silently retire the mika#696 guard for the CI, PR-review and ready-label
/// paths, where it is still load-bearing.
#[test]
fn mika2517_the_fallthrough_domain_stands_the_zero_tools_guard_down() {
    for msg in [
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
        "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko",
        "[GitHub] Issue assigned: senara-solutions/mika#100 — title",
        "[GitHub] Issue closed: senara-solutions/mika#100 — title",
        "[GitHub] discussion.created on senara-solutions/mika",
    ] {
        assert!(
            !webhook_zero_tools_trigger(msg),
            "{msg:?} is Webhook Fallthrough: its prompt contract is to \
                 acknowledge and stop, so re-prompting it for calling no tool is \
                 the engine contradicting itself (mika#2517 AC2)"
        );
    }
}

/// **V4, positive half** — outside the domain the guard is untouched.
#[test]
fn mika2517_the_zero_tools_guard_still_fires_outside_the_domain() {
    for msg in [
        // Ready-label: the dispatch path, where a turn with zero tools is a
        // dispatch that did not happen.
        "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title",
        // qa skill territory.
        "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer",
        // ci skill territory — and note its *success* sibling is still
        // excluded by the mika#1469 literal, which is why that literal stays.
        "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)",
    ] {
        assert!(
            webhook_zero_tools_trigger(msg),
            "{msg:?} is outside the fallthrough domain — mika#2517 must not \
                 retire the mika#696 guard there"
        );
    }
}

// -- mika#2517 U2 — the tool the fallthrough turn is not handed --

/// **V3** — the domain widens the denylist; off the domain the slice is
/// handed through **identically**.
///
/// The `Cow::Borrowed` assertion is the load-bearing half: it is what makes
/// "no regression outside the domain" a property of the type rather than of
/// a comparison that could pass on a fresh allocation with the same content.
#[test]
fn mika2517_effective_disabled_tools_widens_only_on_the_domain() {
    let identity = vec!["run_team".to_string()];

    let off_domain = effective_disabled_tools(&identity, "implement mika#2517");
    assert!(
        matches!(off_domain, std::borrow::Cow::Borrowed(_)),
        "a turn outside the domain must hand the identity slice through \
             untouched — an owned clone here means the nominal path allocates"
    );
    assert_eq!(&*off_domain, identity.as_slice());

    let on_domain = effective_disabled_tools(
        &identity,
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
    );
    assert!(
        on_domain.iter().any(|t| t == "run_team"),
        "the identity denylist must survive the widening"
    );
    assert!(
        on_domain.iter().any(|t| t == "create_task"),
        "a fallthrough turn must not be handed create_task (mika#2517 AC1)"
    );
}

/// mika#2667 AC2 — the `hold[review]` turn handed back with no active task
/// withholds `create_task`; a `block[ac]` handed to the LLM does not.
#[test]
fn mika2667_a_hold_review_without_task_withholds_create_task() {
    let identity: Vec<String> = Vec::new();
    let held = format!(
        "{} on https://github.com/senara-solutions/mika/pull/12. …\n\n\
             [GitHub] PR review (commented) on senara-solutions/mika#12 (t) by @mika-platform-qa",
        crate::server::verdict_handler::HOLD_REVIEW_NO_TASK_MARKER
    );
    assert!(
        effective_disabled_tools(&identity, &held)
            .iter()
            .any(|t| t == "create_task")
    );

    // Negative control: the `block[ac]` passthrough and the raw PR review
    // event keep the nominal, untouched slice.
    for msg in [
        "[verdict_handler] VERDICT: block[ac] on https://github.com/senara-solutions/mika/pull/12 \
             but no active in_progress task found. Passing through to LLM.",
        "[GitHub] PR review (commented) on senara-solutions/mika#12 (t) by @mika-platform-qa",
    ] {
        assert!(
            matches!(
                effective_disabled_tools(&identity, msg),
                std::borrow::Cow::Borrowed(_)
            ),
            "{msg}"
        );
    }
}

/// The widening is idempotent: an identity that already denies the tool
/// does not end up denying it twice.
#[test]
fn mika2517_the_widening_does_not_duplicate_an_existing_entry() {
    let identity = vec!["Create_Task".to_string()];
    let widened = effective_disabled_tools(
        &identity,
        "[GitHub] New comment on senara-solutions/mika#933 by @samidarko",
    );
    assert_eq!(
        widened.len(),
        1,
        "the match is case-insensitive downstream, so re-adding the name \
             would be a duplicate with no effect and a misleading log line: {widened:?}"
    );
}

/// The withheld list and the filter agree on the name (mika#2517).
///
/// `apply_agent_tool_visibility` matches case-insensitively, so this pins
/// the end-to-end path rather than the constant alone: the tool the
/// constant names must be the tool the filter removes.
#[test]
fn mika2517_the_withheld_tool_is_actually_evicted_by_the_filter() {
    let mut defs = vec![
        mika_common::claude::ToolDefinition {
            name: "create_task".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        },
        mika_common::claude::ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        },
    ];
    let disabled = effective_disabled_tools(
        &[],
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
    );
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        !names.contains(&"create_task"),
        "left in the array: {names:?}"
    );
    assert!(
        names.contains(&"run_claude_pilot"),
        "run_claude_pilot stays SERVED on purpose — its gate 0 refusal is the \
             measurement that the model tried, and hiding the tool would delete \
             that signal (mika#2517 § 3.3 (b)): {names:?}"
    );
}

// -- #910 webhook_no_unauthorized_dispatch trigger tests --

#[test]
fn no_unauthorized_dispatch_trigger_matches_comment_events() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] New comment on senara-solutions/mika#906 by @samidarko\nhttps://github.com/senara-solutions/mika/issues/906#issuecomment-123\n\nGroomed end-to-end."
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_matches_non_ready_labels() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled bug on mika#999"
    ));
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled p1-important on mika#999"
    ));
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled enhancement on mika#999"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_skips_pr_review() {
    // PR review events are qa skill territory — must NOT trigger (#1102).
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_skips_check_suite() {
    // Check suite events are ci skill territory — must NOT trigger (#1102).
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Check suite failure on branch fix/foo"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_ready_label() {
    // Ready-label events must NOT trigger this guard — the positive-case
    // guard (webhook_ready_label_dispatch) handles them.
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#999"
    ));
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 \u{2014} title"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_direct_prompts() {
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "implement mika issue#999"
    ));
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "dispatch mika#906"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_empty() {
    assert!(!webhook_no_unauthorized_dispatch_trigger(""));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_callback() {
    // Callback triggers have a different prefix.
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[callback: run_claude_pilot]"
    ));
}

// -- #910 webhook_no_unauthorized_dispatch satisfied tests --

#[test]
fn no_unauthorized_dispatch_satisfied_when_empty() {
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[]));
}

#[test]
fn no_unauthorized_dispatch_satisfied_when_only_run_gh() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_gh".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

#[test]
fn no_unauthorized_dispatch_satisfied_when_pilot_failed() {
    // Failed run_claude_pilot (e.g., task_not_dispatchable) is already
    // blocked by the dispatch-readiness guard — no double-rejection needed.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

#[test]
fn no_unauthorized_dispatch_not_satisfied_when_pilot_succeeded() {
    // This is the unauthorized dispatch case — run_claude_pilot succeeded
    // on a non-ready webhook turn.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "create_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

// -- #910 ordering invariant --

#[test]
fn no_unauthorized_dispatch_guard_ordering() {
    // webhook_no_unauthorized_dispatch must appear AFTER
    // webhook_ready_label_dispatch (which handles the positive case)
    // and BEFORE webhook_zero_tools (logical grouping of webhook guards).
    let labels: Vec<&str> = INTENT_GUARDS.iter().map(|g| g.label).collect();
    let ready_idx = labels
        .iter()
        .position(|l| *l == "webhook_ready_label_dispatch")
        .expect("webhook_ready_label_dispatch must be registered");
    let no_dispatch_idx = labels
        .iter()
        .position(|l| *l == "webhook_no_unauthorized_dispatch")
        .expect("webhook_no_unauthorized_dispatch must be registered");
    let zero_idx = labels
        .iter()
        .position(|l| *l == "webhook_zero_tools")
        .expect("webhook_zero_tools must be registered");
    assert!(
        ready_idx < no_dispatch_idx,
        "webhook_ready_label_dispatch (idx={ready_idx}) must precede \
             webhook_no_unauthorized_dispatch (idx={no_dispatch_idx})"
    );
    assert!(
        no_dispatch_idx < zero_idx,
        "webhook_no_unauthorized_dispatch (idx={no_dispatch_idx}) must precede \
             webhook_zero_tools (idx={zero_idx})"
    );
}

#[test]
fn intent_guards_registry_count() {
    // Six guards after adding deferred_dispatch_action (mika#1011).
    assert_eq!(
        INTENT_GUARDS.len(),
        6,
        "INTENT_GUARDS should have exactly 6 entries"
    );
}

// -- load_gated_summary tests (Axis 3 — mika#1021) --

/// Seed a summary into the async DB by saving enough messages and compacting.
async fn seed_summary(db: &AsyncDatabase, content: &str) {
    // Save enough messages so replace_with_summary has something to compact.
    for i in 0..5 {
        db.save_message("test-session", "user", &format!("msg {i}"), None)
            .await
            .unwrap();
    }
    let old = db.load_messages_before_window(3).await.unwrap();
    let highest_id = old.last().unwrap().id;
    db.replace_with_summary(content, highest_id).await.unwrap();
}

fn make_callback_trigger() -> SilentTrigger {
    SilentTrigger::Callback {
        task_id: "test-task".to_string(),
        label: "test".to_string(),
        result: "done".to_string(),
        failed: false,
        parent_task_id: None,
    }
}

#[tokio::test]
async fn gated_summary_no_cap_silent_returns_full() {
    // max_tokens = None, silent turn, summary present → returns full summary (regression).
    let db = test_async_db();
    seed_summary(&db, "Full summary content here").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: None,
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, Some("Full summary content here".to_string()));
}

#[tokio::test]
async fn gated_summary_zero_sentinel_silent_returns_none() {
    // max_tokens = Some(0), silent turn → returns None (load-omit sentinel).
    let db = test_async_db();
    seed_summary(&db, "Should be omitted").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(0),
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn gated_summary_zero_sentinel_non_silent_returns_full() {
    // max_tokens = Some(0), non-silent turn → returns full (cap gated by mode).
    let db = test_async_db();
    seed_summary(&db, "Full content non-silent").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(0),
    };
    let result = load_gated_summary(&db, &config, None).await.unwrap();
    assert_eq!(result, Some("Full content non-silent".to_string()));
}

#[tokio::test]
async fn gated_summary_cap_truncates_in_silent() {
    // max_tokens = Some(n), silent turn, summary > n tokens → truncated.
    let db = test_async_db();
    let long_summary = "word ".repeat(500); // 2500 chars, way over budget
    seed_summary(&db, &long_summary).await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(100), // 400 char budget
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    let content = result.unwrap();
    assert!(content.contains("[… summary truncated to fit silent-mode budget …]"));
    // Total truncated content (before marker) should be ≤ 400 chars
    let before_marker = content
        .strip_suffix("\n[… summary truncated to fit silent-mode budget …]")
        .unwrap();
    assert!(before_marker.len() <= 400);
}

#[tokio::test]
async fn gated_summary_cap_under_budget_returns_full() {
    // max_tokens = Some(n), silent turn, summary < n tokens → full summary.
    let db = test_async_db();
    seed_summary(&db, "Short summary").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(1000), // 4000 char budget, summary is tiny
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, Some("Short summary".to_string()));
}

#[tokio::test]
async fn gated_summary_axis4_wins_over_axis3() {
    // Axis 4 inject = false + any max_tokens → returns None without DB call.
    let db = test_async_db();
    // Don't even seed a summary — Axis 4 should short-circuit before DB call.

    let config = prompt::ContextSummaryConfig {
        inject: false,
        max_tokens: Some(500),
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn gated_summary_no_summary_stored() {
    // inject = true, max_tokens = None, no summary in DB → returns None.
    let db = test_async_db();
    // No summary seeded.

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: None,
    };
    let result = load_gated_summary(&db, &config, None).await.unwrap();
    assert_eq!(result, None);
}

// -- Compact provider gate tests (mika#1491) --

#[test]
fn test_inject_skills_compact_provider_filters_tools() {
    // When is_compact_provider = true: no skill prompts appended,
    // tools filtered to COMPACT_PROVIDER_CORE_TOOLS only, per_skill_bytes empty.
    let tools = crate::tools::default_tools();
    let mut entry = make_skill_entry("self-dev", 30, &["run_claude_pilot"]);
    entry.prompt_snippet = "A very large skill prompt that should NOT appear.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base compact prompt.".to_string();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, variant, per_skill_bytes) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "mikamodel",
        "mika",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        true, // compact provider
    );

    // No skill prompts should be appended
    assert!(
        !system.contains("<context type=\"skill\""),
        "compact provider should skip all skill prompts"
    );
    assert!(
        !system.contains("self-dev Skill"),
        "compact provider should not inject skill name"
    );

    // Only core tools should be present
    let tool_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for core_tool in COMPACT_PROVIDER_CORE_TOOLS {
        assert!(
            tool_names.contains(core_tool),
            "core tool '{core_tool}' should be in compact tool set"
        );
    }
    assert!(
        defs.len() <= COMPACT_PROVIDER_CORE_TOOLS.len(),
        "compact provider should have at most {} tools, got {}",
        COMPACT_PROVIDER_CORE_TOOLS.len(),
        defs.len()
    );
    // Skill-defined tools should NOT be present
    assert!(
        !tool_names.contains(&"run_claude_pilot"),
        "skill-defined tools should be excluded in compact mode"
    );

    // No variant info (skills skipped)
    assert!(variant.is_none());
    // No per-skill bytes
    assert!(per_skill_bytes.is_empty());
}

#[test]
fn test_inject_skills_non_compact_provider_unchanged() {
    // When is_compact_provider = false: behavior unchanged — skills appended, all tools present.
    let tools = crate::tools::default_tools();
    let mut entry = make_skill_entry("test-skill", 30, &["test_tool"]);
    entry.prompt_snippet = "Use this skill.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base prompt.".to_string();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, variant, per_skill_bytes) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        false, // non-compact
    );

    // Skills should be appended
    assert!(
        system.contains("<context type=\"skill\""),
        "non-compact provider should include skill prompts"
    );
    assert!(system.contains("test-skill Skill"));

    // All default tools + skill tools should be present
    assert!(
        defs.len() > COMPACT_PROVIDER_CORE_TOOLS.len(),
        "non-compact should have more tools than core set"
    );
    let tool_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        tool_names.contains(&"test_tool"),
        "skill tools should be included"
    );

    // Variant and per_skill_bytes should be populated
    assert!(variant.is_some());
    assert!(!per_skill_bytes.is_empty());
}

#[test]
fn test_compact_provider_core_tools_are_valid() {
    // Every name in COMPACT_PROVIDER_CORE_TOOLS must exist in default_tools().
    // Prevents drift when tools are renamed or removed.
    let tools = crate::tools::default_tools();
    let tool_names: std::collections::HashSet<String> =
        tools.definitions().iter().map(|d| d.name.clone()).collect();

    for &core_tool in COMPACT_PROVIDER_CORE_TOOLS {
        assert!(
            tool_names.contains(core_tool),
            "COMPACT_PROVIDER_CORE_TOOLS entry '{core_tool}' not found in default_tools() — \
                 was it renamed or removed?"
        );
    }
}

// ===========================================================================
// mika#1889 — Per-turn usage/token logging (RT-005 brick 4/5)
//
// Structural coverage of the `build_turn_usage_fields` pure builder. The
// token/dimension mapping is tested directly here so we do not depend on a
// captured tracing subscriber (D5). The emission site itself is exercised
// no-panic by the existing eval harness in `tests/eval/`.
// ===========================================================================

fn usage_with_cache() -> LlmUsage {
    LlmUsage {
        input_tokens: 1234,
        output_tokens: 567,
        cache_creation_input_tokens: Some(89),
        cache_read_input_tokens: Some(4321),
    }
}

fn usage_without_cache() -> LlmUsage {
    LlmUsage {
        input_tokens: 10,
        output_tokens: 20,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    }
}

#[test]
fn build_turn_usage_success_with_cache_passes_through_tokens() {
    let u = usage_with_cache();
    let f = build_turn_usage_fields(
        3,
        Some(&u),
        "ToolUse",
        true,
        "success",
        250,
        None,
        None,
        None,
    );
    assert_eq!(f.step, 3);
    assert_eq!(f.input_tokens, 1234);
    assert_eq!(f.output_tokens, 567);
    // Anthropic-only cache fields: `Option<u64>` flattened to `0` when absent —
    // here `Some(4321)` and `Some(89)` are preserved verbatim.
    assert_eq!(f.cache_read_tokens, 4321);
    assert_eq!(f.cache_write_tokens, 89);
    assert_eq!(f.latency_ms, 250);
    assert_eq!(f.stop_reason, "ToolUse");
    assert!(f.tool_use_in_turn);
    assert_eq!(f.status, "success");
}

#[test]
fn build_turn_usage_success_no_cache_flattens_option_to_zero() {
    // Non-Anthropic providers report None for cache_creation; some
    // OpenAI-compat providers also report None for cache_read. The log MUST
    // still be jq-parseable unconditionally — `None` → `0`, never a missing
    // field. This is the load-bearing analyzer-shape invariant.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert_eq!(f.input_tokens, 10);
    assert_eq!(f.output_tokens, 20);
    assert_eq!(f.cache_read_tokens, 0);
    assert_eq!(f.cache_write_tokens, 0);
    assert_eq!(f.latency_ms, 0);
    assert!(!f.tool_use_in_turn);
}

#[test]
fn build_turn_usage_error_arm_zeros_all_tokens() {
    // Error/timeout arms have no `LlmUsage`. R3 mandates the event still
    // fires so the covariable "turns" count is not silently undercounted —
    // the tokens roll to zero but the row exists.
    let f = build_turn_usage_fields(7, None, "error", false, "error", 42, None, None, None);
    assert_eq!(f.step, 7);
    assert_eq!(f.input_tokens, 0);
    assert_eq!(f.output_tokens, 0);
    assert_eq!(f.cache_read_tokens, 0);
    assert_eq!(f.cache_write_tokens, 0);
    // latency_ms is preserved even on the error arm — the transport spent
    // wall-clock even if it produced no tokens.
    assert_eq!(f.latency_ms, 42);
    assert_eq!(f.stop_reason, "error");
    assert!(!f.tool_use_in_turn);
    assert_eq!(f.status, "error");
}

#[test]
fn build_turn_usage_continuation_step_sentinel_preserved() {
    // The continuation turn uses `u32::MAX` as the step sentinel in both
    // the DB path (`save_continuation_llm_call`) and the log surface, so
    // the offline analyzer (brick 5/5) can distinguish continuation-turn
    // usage from in-loop step indices without a separate flag.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        u32::MAX,
        Some(&u),
        "EndTurn",
        false,
        "success",
        100,
        None,
        None,
        None,
    );
    assert_eq!(f.step, u32::MAX);
}

#[test]
fn build_turn_usage_tool_use_boolean_is_raw_observation() {
    // Prime hard condition #1 (planning-tokens = SEUL primary outcome):
    // `tool_use_in_turn` is a RAW mechanical observation — did the model
    // request tool calls this turn — not a graded planning/execution
    // classification. Verified here by exercising both truth values with
    // otherwise-identical inputs.
    let u = usage_without_cache();
    let f_true =
        build_turn_usage_fields(1, Some(&u), "ToolUse", true, "success", 0, None, None, None);
    let f_false = build_turn_usage_fields(
        1,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert!(f_true.tool_use_in_turn);
    assert!(!f_false.tool_use_in_turn);
    // No `phase`/`is_planning`/`role` field exists on the struct — D1/R5
    // enforced by absence (the field simply does not exist). This test
    // documents that invariant structurally.
}

#[test]
fn build_turn_usage_latency_ms_is_raw_pass_through() {
    // latency_ms is R1's turn-cost covariable. It's a raw transport
    // measurement (wall-clock of the HTTP call), not an estimand
    // component. Verified here as a pure pass-through.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        12345,
        None,
        None,
        None,
    );
    assert_eq!(f.latency_ms, 12345);
}

// ===========================================================================
// mika#2331 (AC1) — the turn says its size, including when it fails
// ===========================================================================

/// The load-bearing distinction of D6: `None` and `Some(0)` are different
/// answers. No request is empty, so a `0` would be a readable lie about a
/// measurement that did not happen; `null` says "not measured".
#[test]
fn mika2331_request_bytes_none_and_some_do_not_collapse() {
    let u = usage_without_cache();
    let measured = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        Some(59_812),
        Some(48_000),
        None,
    );
    assert_eq!(measured.request_bytes, Some(59_812));
    assert_eq!(measured.system_prompt_bytes, Some(48_000));

    let unmeasured = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert_eq!(unmeasured.request_bytes, None);
    assert_eq!(unmeasured.system_prompt_bytes, None);
    assert_ne!(unmeasured.request_bytes, Some(0));
}

/// The arm the whole of AC1 is about. A hung call returns no `usage`, so
/// every token count is zero — and until now that zero was the line's only
/// statement about how big the brief was. The two sizes are measured before
/// the call and must survive it.
#[test]
fn mika2331_error_arm_still_carries_the_brief_size() {
    let f = build_turn_usage_fields(
        4,
        None,
        "error",
        false,
        "error",
        420_000,
        Some(59_812),
        Some(48_000),
        None,
    );
    assert_eq!(f.input_tokens, 0, "no usage on the error arm — unchanged");
    assert_eq!(
        f.request_bytes,
        Some(59_812),
        "the size of a brief that timed out is exactly what a 420 s hang needs to be read"
    );
    assert_eq!(f.system_prompt_bytes, Some(48_000));
}

// ===========================================================================
// mika#1671 (AC2) — delegation-outcome classifier ruleset
//
// D2 is strict transport-only. The classifier must qualify: HTTP 503 (founding
// incident), transport/connection/dns error substrings, and deadline-exceeded
// (mapped from `TeamAgentOutcome::TimedOut`). It must NOT qualify: apologetic
// business responses that arrive as a successful `Ok(response)`, and clean
// successes — those route to `Completed`/`BusinessLogicFailure` so a mixed
// iteration never short-circuits (AC4).
// ===========================================================================

/// Build a `ToolCallSummary` stub for classifier tests. `success == false`
/// mirrors a `ToolOutput::error` result (the only shape the classifier inspects).
fn summary(name: &str, output: &str, success: bool) -> ToolCallSummary {
    ToolCallSummary {
        step: 0,
        name: name.to_string(),
        input_summary: String::new(),
        output_summary: output.to_string(),
        success,
        non_zero_exit: false,
    }
}

#[test]
fn classify_503_from_a2a_call_qualifies() {
    // Founding-incident shape: a2a_call 503 surfaced as a failed tool result.
    let summaries = vec![summary(
        "a2a_call",
        "HTTP status server error (503 Service Unavailable) for url (...)",
        false,
    )];
    assert!(matches!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_transport_error_substring_qualifies() {
    let summaries = vec![summary("a2a_call", "transport error: broken pipe", false)];
    assert!(matches!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_connection_refused_and_dns_error_qualify() {
    for sig in ["connection refused", "dns error while resolving host"] {
        let summaries = vec![summary("a2a_call", sig, false)];
        assert!(
            matches!(
                classify_delegation_from_summaries(&summaries),
                DelegationOutcome::TransportError { .. }
            ),
            "signature {sig:?} must qualify as transport failure"
        );
    }
}

#[test]
fn classify_deadline_exceeded_qualifies_via_timed_out() {
    // A hard agent-loop deadline surfaces as `TeamAgentOutcome::TimedOut`, which
    // maps to a retryable transport error per D2 (deadline-exceeded qualifies).
    let outcome = TeamAgentOutcome::TimedOut(
        "Agent timed out while processing team task (deadline exceeded in run_loop).".to_string(),
    );
    assert!(matches!(
        outcome.delegation_outcome(),
        DelegationOutcome::TransportError {
            retryable: true,
            ..
        }
    ));
}

#[test]
fn classify_apologetic_business_response_does_not_qualify() {
    // The model said "I can't" but every tool call succeeded — a business-logic
    // apology arrives as `Ok(response)` and must NOT be a transport failure (AC4).
    let summaries = vec![
        summary("read_file", "file contents ...", true),
        summary(
            "send_message",
            "I'm sorry, I can't complete this task with the available data.",
            true,
        ),
    ];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_clean_success_does_not_qualify() {
    let summaries = vec![summary("read_file", "ok", true)];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_empty_summaries_is_completed() {
    assert_eq!(
        classify_delegation_from_summaries(&[]),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_successful_call_with_503_in_output_does_not_qualify() {
    // Only FAILED calls are inspected: a successful result that merely mentions
    // "503" in prose (e.g. the agent describing a status code) is not a transport
    // failure — guards against false positives on business content.
    let summaries = vec![summary(
        "send_message",
        "The upstream returned 503 earlier but recovered; here is the summary.",
        true,
    )];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_error_string_transport_shaped_qualifies() {
    // Hard agent-loop `Err` with a transport-shaped string still counts.
    assert!(matches!(
        classify_delegation_from_error("connection refused by peer"),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_error_string_non_transport_is_business_failure() {
    // A hard `Err` that is not transport-shaped must NOT contribute to the
    // all-transport short-circuit (AC4) — it maps to BusinessLogicFailure.
    assert_eq!(
        classify_delegation_from_error("agent 'foo' not found in team resources"),
        DelegationOutcome::BusinessLogicFailure
    );
}

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

// -- mika#2814 : publier des outils MCP passe par le helper qui ensemence --

/// Lecteurs de production qui lisent l'inventaire MCP **sans** le publier au
/// modèle, en `(fichier, aiguille de la ligne, raison du droit)`.
///
/// **Née non vide, et ce n'est pas un emplacement pour le prochain oubli.**
/// Ce n'est pas l'allowlist que mika#2309 condamne : `tool_definitions()`
/// est un getter, et lire l'inventaire sans le verser dans la requête LLM
/// est une chose qu'on a légitimement le droit de faire. Cette liste est
/// donc un **inventaire de droits**, et **aucun ticket de suite n'est dû
/// pour ses deux entrées** — un relecteur n'a pas à chercher le `mika#NNNN`
/// qui les retirerait, il n'y en a pas. Ce que la garde exige à la place,
/// c'est qu'elles restent **vraies** : son assertion auto-nettoyante rougit
/// sur une entrée que le balayage ne retrouve plus.
///
/// **L'aiguille est le texte de la ligne, pas le seul fichier**, parce que
/// `agent_loop/mod.rs` porte aussi [`publier_outils_mcp`] et les deux
/// boucles d'agent : exempter le fichier entier y rendrait un troisième site
/// de publication invisible, à l'endroit précis où il serait écrit. Et
/// keyer sur le texte échoue dans le bon sens — une ligne réécrite ne
/// retrouve plus son exemption, donc la garde **rougit**. C'est l'inverse
/// exact du mode de panne de l'ancienne garde, dont l'aiguille *détectait*
/// la faute par sa forme et la laissait passer dès qu'on l'écrivait
/// autrement.
const LECTEURS_NON_PUBLIANTS_MCP: &[(&str, &str, &str)] = &[
    (
        "agent_loop/mod.rs",
        "flat_map(|m| m.tool_definitions().iter().map(|d| d.name.clone()))",
        "collecte les NOMS des outils disponibles (`available_tool_names`) \
             pour les gardes de fin de tour ; aucune définition n'entre dans le \
             lot envoyé au modèle",
    ),
    (
        "kg/domain_builder.rs",
        "for tool_def in mcp.tool_definitions()",
        "projette les outils MCP en entités du graphe de connaissance ; la \
             destination est SQLite, pas la requête LLM",
    ),
];

/// L'entrée de [`LECTEURS_NON_PUBLIANTS_MCP`] qui couvre `(rel, texte)`.
///
/// Écrite une seule fois pour que le contrôle négatif ci-dessous exerce le
/// prédicat de la garde, et non une copie qui pourrait en diverger.
fn lecteur_declare(rel: &str, texte: &str) -> Option<usize> {
    LECTEURS_NON_PUBLIANTS_MCP
        .iter()
        .position(|(fichier, aiguille, _)| rel.contains(fichier) && texte.contains(aiguille))
}

/// Lignes de source retenues par un balayage, en `(ligne 1-based, texte)`.
type LignesRetenues = Vec<(usize, String)>;

/// Bornes `(début, fin)` 1-based inclusives du corps de
/// [`publier_outils_mcp`] dans `src`, s'il y figure.
///
/// L'exclusion du corps du helper est **de portée, pas de fichier** : c'est
/// elle qui laisse la garde mordre sur un troisième site ajouté dans
/// `agent_loop/mod.rs` lui-même — le fichier le plus probable de tous,
/// puisque c'est là que vivent les boucles d'agent. Les bornes tiennent à
/// deux invariants que `cargo fmt --check` maintient en CI : une fonction
/// libre s'ouvre en colonne 0, et son corps se ferme sur un `}` en
/// colonne 0. Si un jour l'un des deux tombe, la fonction rend `None`, le
/// corps cesse d'être exclu et la garde **rougit** : échec fermé.
fn bornes_du_helper_de_publication(src: &str) -> Option<(usize, usize)> {
    let lignes: Vec<&str> = src.lines().collect();
    let debut = lignes
        .iter()
        .position(|l| l.starts_with("fn publier_outils_mcp("))?;
    let fin = lignes[debut + 1..].iter().position(|l| *l == "}")? + debut + 1;
    Some((debut + 1, fin + 1))
}

/// Usages de production de l'inventaire MCP, en `(ligne 1-based, texte)` :
/// les lignes qui **lisent** `tool_definitions()`, et celles qui
/// **appellent** [`publier_outils_mcp`] hors de sa définition.
///
/// L'aiguille de lecture est `tool_definitions()` **nue**, là où mika#1959
/// visait la forme `extend_from_slice(mcp.tool_definitions())`. Ce
/// changement est le cœur du vol 2 : viser la forme, c'était laisser passer
/// une publication écrite autrement (une boucle, un `chain`, un `clone()`
/// intermédiaire). Viser l'inventaire n'est devenu possible qu'avec le
/// helper, qui crée **un** site où la lecture publiante est légitime ; son
/// corps est donc retiré de la liste, par portée, via
/// [`bornes_du_helper_de_publication`]. Les autres lecteurs légitimes sont
/// déclarés par [`LECTEURS_NON_PUBLIANTS_MCP`], que la garde confronte au
/// balayage.
///
/// L'aiguille d'appel exclut la **définition** (`fn publier_outils_mcp(`) :
/// la déclarer n'est pas l'appeler, et la compter rapprocherait le compte de
/// deux avec un seul appel réel.
///
/// Les deux exclusions de [`identity_history_reader_sites`] se composent
/// identiquement : un fichier dont le **chemin** est du test n'a pas de
/// moitié production (mika#2321), et partout ailleurs les régions de test
/// sont **masquées** plutôt que tronquées (mika#2398) — ce qui est
/// porteur ici, puisque les deux aiguilles figurent en clair dans les
/// fixtures du contrôle négatif ci-dessous, dans ce même fichier.
fn usages_des_definitions_mcp(
    path: &std::path::Path,
    src: &str,
) -> (LignesRetenues, LignesRetenues) {
    if crate::source_scan::is_test_source_path(path) {
        return (Vec::new(), Vec::new());
    }
    let masque = mika_common::source_guard::mask_test_regions(src);
    let corps_du_helper = bornes_du_helper_de_publication(&masque);
    let lignes: LignesRetenues = masque
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim().to_string()))
        .filter(|(_, line)| !line.starts_with("//") && !line.starts_with('*'))
        .collect();
    let lectures = lignes
        .iter()
        .filter(|(n, line)| {
            line.contains("tool_definitions()")
                && !corps_du_helper.is_some_and(|(d, f)| *n >= d && *n <= f)
        })
        .cloned()
        .collect();
    let appels = lignes
        .iter()
        .filter(|(_, line)| {
            line.contains("publier_outils_mcp(") && !line.contains("fn publier_outils_mcp(")
        })
        .cloned()
        .collect();
    (lectures, appels)
}

/// **mika#2814** — publier des outils MCP au modèle passe par
/// [`publier_outils_mcp`], qui ensemence la map L4 dans le même appel.
///
/// **Le nom a changé parce que la propriété a changé.** mika#1959 appariait
/// deux **textes** — `extend_from_slice(mcp.tool_definitions())` d'un côté,
/// `seed_mcp_data_grades(` de l'autre — et une publication écrite autrement
/// (une boucle, un `chain`, un `clone()` intermédiaire) laissait la garde
/// verte en rouvrant le contournement F5 de mika#1798. Ce que cette garde
/// prouve désormais est plus fort : il n'existe **qu'un seul chemin**, et
/// deux sites l'empruntent.
///
/// Trois assertions, chacune fermant une moitié que les autres laissent
/// ouverte :
///
/// 1. **Le compte d'appels est deux** — la conversation et l'agent
///    d'équipe. Le site silencieux/callback/heartbeat (`run_silent_agent`)
///    est **hors population par construction** : il reçoit `None`, donc il
///    ne publie ni n'ensemence. C'est pourquoi le nombre attendu est deux et
///    non trois.
/// 2. **Aucune lecture de `tool_definitions()`** hors du corps du helper et
///    hors de [`LECTEURS_NON_PUBLIANTS_MCP`]. Sans elle, un troisième site
///    écrivant directement `defs.extend_from_slice(mcp.tool_definitions())`
///    laisserait le compte à deux et passerait. Cette assertion était
///    **inécrivable avant le helper** : il n'existait alors aucun site
///    unique où la lecture publiante était légitime, donc l'aiguille devait
///    viser la forme plutôt que l'inventaire.
/// 3. **L'inventaire des lecteurs est auto-nettoyant** — chaque entrée doit
///    correspondre à un usage que le balayage retrouve. Une entrée dont la
///    ligne a disparu ou a été réécrite rougit, donc la liste ne peut pas
///    survivre à ce qu'elle décrit (mika#2309 : le geste qui rend une
///    exception inutile est celui qui la fait expirer).
///
/// Ce que cette garde ne dit **pas** : que les deux sites publient le *bon*
/// lot. Le filtre L2 de `McpManager::connect_all`
/// ([`crate::mcp::retain_publiables`], vol 1 de mika#2814) est **en amont**
/// d'eux, donc aucun n'a bougé et la map L4 garde l'inventaire **complet**
/// (grades et routage ne sont pas amputés) — ce que l'injection
/// « passe-plat » de `todos/2814-injection-verification.md` § Cas 4 a mesuré
/// plutôt que supposé. Et un appel sur un nom non publié n'est **pas**
/// « outil inconnu » : voir § *La porte que ce helper tient est une porte*
/// sur [`publier_outils_mcp`], et § Cas 1 de
/// `todos/2814-vol2-rouge-mesure.md` qui l'a mesuré.
#[test]
fn mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence() {
    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let src_root = scanner.src_root().to_path_buf();
    let mut appels: Vec<String> = Vec::new();
    let mut lectures_non_declarees: Vec<String> = Vec::new();
    let mut entrees_retrouvees = vec![false; LECTEURS_NON_PUBLIANTS_MCP.len()];

    scanner.for_each(|path, production| {
        let rel = path
            .strip_prefix(&src_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let (lectures, appels_ici) = usages_des_definitions_mcp(path, production);
        for (line, text) in appels_ici {
            appels.push(format!("{rel}:{line}: {text}"));
        }
        for (line, text) in lectures {
            match lecteur_declare(&rel, &text) {
                Some(i) => entrees_retrouvees[i] = true,
                None => lectures_non_declarees.push(format!("{rel}:{line}: {text}")),
            }
        }
    });

    assert_eq!(
        appels.len(),
        2,
        "mika#2814 — exactement deux sites appellent `publier_outils_mcp` : la \
             conversation et l'agent d'équipe. Le mode silencieux est hors population \
             (il reçoit `None`). Trouvés :\n{}",
        appels.join("\n")
    );
    assert!(
        lectures_non_declarees.is_empty(),
        "mika#2814 — ces lignes de production lisent `mcp.tool_definitions()` hors \
             du corps de `publier_outils_mcp` et hors de `LECTEURS_NON_PUBLIANTS_MCP` \
             :\n{}\n\nSi c'est une publication au modèle, passez par \
             `publier_outils_mcp` : il ensemence la map L4 dans le même appel, et un \
             outil MCP testimony publié sans grade laisse `mcp.call_tool` partir sur \
             n'importe quel nom obtenu autrement que par l'inventaire publié — c'est le \
             contournement F5 de mika#1798, mesuré au § Cas 1 de \
             `todos/2814-vol2-rouge-mesure.md`. Si c'est une lecture qui ne publie pas, \
             déclarez-la dans `LECTEURS_NON_PUBLIANTS_MCP` avec la raison de son droit : \
             aucun ticket de suite n'est dû pour un droit légitime.",
        lectures_non_declarees.join("\n")
    );
    let perimees: Vec<String> = LECTEURS_NON_PUBLIANTS_MCP
        .iter()
        .zip(&entrees_retrouvees)
        .filter(|(_, retrouvee)| !**retrouvee)
        .map(|((fichier, aiguille, _), _)| format!("{fichier} :: {aiguille}"))
        .collect();
    assert!(
        perimees.is_empty(),
        "mika#2814 — ces entrées de `LECTEURS_NON_PUBLIANTS_MCP` ne correspondent \
             plus à aucune ligne que le balayage retrouve :\n{}\n\nLe lecteur a été \
             retiré, déplacé ou réécrit : retirez l'entrée, ou corrigez son aiguille. \
             Une exemption qui survit à ce qu'elle décrit est un droit que plus personne \
             ne possède, et c'est par là qu'une garde cesse de garder (mika#2309).",
        perimees.join("\n")
    );
}

/// **mika#2814** — un seul appel à [`publier_outils_mcp`] publie les outils
/// MCP au modèle **et** les verse dans la map L4.
///
/// C'est la propriété que le vol 2 achète : avant, les deux gestes étaient
/// deux lignes qu'une garde appariait par comparaison de chaînes, et
/// n'écrire que la première était un défaut écrivable. Ce test mesure les
/// deux moitiés sur le **même** appel — c'est-à-dire qu'il échouerait sur
/// un helper qui n'en ferait qu'une, là où la garde de source, elle, ne
/// verrait rien (elle compte des appels, pas des effets).
#[test]
fn mika2814_un_seul_appel_publie_et_ensemence() {
    use crate::skills::manifest::DataGrade;

    let outil_mcp = "mcp__journal-perso__read_entry";
    let mcp = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]),
    );

    let mut defs: Vec<mika_common::claude::ToolDefinition> = Vec::new();
    let mut grades = HashMap::from([("web_search".to_string(), DataGrade::Operational)]);
    publier_outils_mcp(&mut defs, &mut grades, Some(&mcp));

    assert_eq!(
        defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        mcp.tool_definitions()
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        "l'appel doit verser au modèle exactement l'inventaire publiable du manager"
    );
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "et le même appel doit avoir ensemencé la map L4 — c'est la soudure que ce \
             helper existe pour rendre indissociable"
    );
    assert_eq!(
        grades.get("web_search"),
        Some(&DataGrade::Operational),
        "sans toucher aux entrées des skills"
    );
}

/// **mika#2814** — `None` laisse les deux arguments intacts.
///
/// Le mode silencieux / callback / heartbeat est **hors population par
/// construction**, et c'est pourquoi la garde de source attend deux appels
/// et non trois. Ce test est ce qui rend cette phrase vraie plutôt
/// qu'affirmée : sans manager, ni le lot d'outils ni la map ne bougent.
#[test]
fn mika2814_sans_manager_le_helper_ne_touche_a_rien() {
    use crate::skills::manifest::DataGrade;

    let map_skills = || HashMap::from([("web_search".to_string(), DataGrade::Operational)]);
    let mut defs: Vec<mika_common::claude::ToolDefinition> = Vec::new();
    let mut grades = map_skills();
    publier_outils_mcp(&mut defs, &mut grades, None);

    assert!(
        defs.is_empty(),
        "sans manager, aucune définition n'est publiée au modèle"
    );
    assert_eq!(grades, map_skills(), "et la map L4 reste celle des skills");
}

/// **mika#1959** — `seed_mcp_data_grades` verse les grades MCP dans la map
/// L4 sans toucher aux entrées des skills, ne rabat jamais un `Testimony`
/// sur collision de nom, et `None` (mode silencieux) laisse la map intacte.
///
/// **Ce test reste, et son objet s'est précisé** (mika#2814 vol 2). Il ne
/// tient plus la moitié « l'appel est présent au site » — c'est désormais
/// [`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`] et
/// [`mika2814_un_seul_appel_publie_et_ensemence`] qui s'en chargent. Ce
/// qu'il tient, et que ni l'un ni l'autre ne mesure, c'est la **règle de
/// collision** : elle est une propriété de l'ensemencement seul, et c'est la
/// raison pour laquelle [`seed_mcp_data_grades`] n'est pas fondue dans
/// [`publier_outils_mcp`]. Un corps vidé laisserait les deux gardes vertes.
#[test]
fn mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills() {
    use crate::skills::manifest::DataGrade;

    let outil_mcp = "mcp__journal-perso__read_entry";
    let mcp = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]),
    );
    let map_skills = || HashMap::from([("web_search".to_string(), DataGrade::Operational)]);

    let mut grades = map_skills();
    seed_mcp_data_grades(&mut grades, Some(&mcp));
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "l'outil MCP doit entrer dans la map L4 avec le grade de son serveur"
    );
    assert_eq!(
        grades.get("web_search"),
        Some(&DataGrade::Operational),
        "l'ensemencement MCP ne doit pas toucher aux entrées des skills"
    );
    assert_eq!(grades.len(), 2);

    // Collision : un outil de skill testimony au nom namespacé MCP n'est
    // pas rabattu par un serveur qui n'a rien déclaré.
    let operational = McpManager::sans_connexion_pour_tests(
        HashMap::from([(
            outil_mcp.to_string(),
            ("journal-perso".to_string(), "read_entry".to_string()),
        )]),
        HashMap::from([(outil_mcp.to_string(), DataGrade::Operational)]),
    );
    let mut grades = HashMap::from([(outil_mcp.to_string(), DataGrade::Testimony)]);
    seed_mcp_data_grades(&mut grades, Some(&operational));
    assert_eq!(
        grades.get(outil_mcp),
        Some(&DataGrade::Testimony),
        "sur collision, le grade le plus strict l'emporte — un serveur MCP \
             muet ne rabat jamais un `Testimony` de skill"
    );

    let mut grades = map_skills();
    seed_mcp_data_grades(&mut grades, None);
    assert_eq!(
        grades,
        map_skills(),
        "sans manager (mode silencieux), la map reste celle des skills"
    );
}

/// **Contrôle négatif de bonne foi** pour
/// [`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`].
///
/// Une garde dont le prédicat ne regarde rien est verte, et c'est le mode de
/// défaillance qu'un contrôle négatif existe pour rendre visible. Trois cas,
/// les mêmes que ceux dont le rouge est enregistré dans
/// `todos/2814-vol2-rouge-mesure.md` :
///
/// 1. une publication écrite hors du helper — y compris **sous une forme que
///    l'ancienne aiguille ne voyait pas** ;
/// 2. une lecture nue dans un fichier non déclaré ;
/// 3. l'état livré — définition du helper et deux lecteurs déclarés — qui
///    doit être **vert**, et dont la verdeur repose sur les trois exclusions
///    (portée du helper, prose, régions et chemins de test).
#[test]
fn mika2814_la_garde_du_helper_rougit_sur_une_publication_hors_chemin() {
    let ailleurs = std::path::Path::new("src/ailleurs.rs");

    // --- Cas 1 : un troisième site publie sans passer par le helper. -----
    let hors_helper = "fn troisieme_site(mcp: &McpManager, defs: &mut Vec<ToolDefinition>) {\n    \
             defs.extend_from_slice(mcp.tool_definitions());\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, hors_helper);
    assert!(
        appels.is_empty(),
        "ce site ne passe pas par le helper — c'est tout le défaut"
    );
    assert_eq!(lectures.len(), 1, "et le prédicat doit voir sa lecture");
    assert!(
        lecteur_declare("ailleurs.rs", &lectures[0].1).is_none(),
        "qu'aucun lecteur déclaré ne couvre : c'est le rouge de l'assertion 2"
    );

    // La même publication écrite en boucle. L'aiguille de mika#1959
    // (`extend_from_slice(mcp.tool_definitions())`) ne la voyait pas, et
    // c'est précisément le trou que le vol 2 ferme : l'aiguille porte
    // désormais sur l'inventaire, pas sur la forme qui le verse.
    let en_boucle = "fn troisieme_site(mcp: &McpManager, defs: &mut Vec<ToolDefinition>) {\n    \
             for d in mcp.tool_definitions() {\n        defs.push(d.clone());\n    }\n}\n";
    assert!(
        !en_boucle.contains("extend_from_slice(mcp.tool_definitions())"),
        "la fixture doit bien être invisible à l'ancienne aiguille, sinon ce cas \
             ne mesure rien"
    );
    let (lectures, _) = usages_des_definitions_mcp(ailleurs, en_boucle);
    assert_eq!(
        lectures.len(),
        1,
        "le nouveau prédicat voit la publication en boucle, que l'ancien manquait"
    );

    // --- Cas 2 : une lecture nue dans un fichier non déclaré. ------------
    // Légitime ou non, elle doit être *déclarée* : c'est la déclaration qui
    // porte la raison du droit, et l'absence de raison est le défaut.
    let inconnu = std::path::Path::new("src/nouveau/module.rs");
    let lecture_nue =
        "let noms: Vec<_> = mcp.tool_definitions().iter().map(|d| d.name.clone()).collect();\n";
    let (lectures, _) = usages_des_definitions_mcp(inconnu, lecture_nue);
    assert_eq!(lectures.len(), 1, "le prédicat doit voir la lecture nue");
    assert!(
        lecteur_declare("nouveau/module.rs", &lectures[0].1).is_none(),
        "dans un fichier non déclaré, elle rougit"
    );

    // --- Cas 3 : l'état livré est vert. ---------------------------------
    // 3a. Le corps du helper est exclu par **portée**, et sa définition
    //     n'est pas un appel.
    let helper = "fn publier_outils_mcp(\n    \
             defs: &mut Vec<ToolDefinition>,\n    \
             grades: &mut HashMap<String, DataGrade>,\n    \
             mcp: Option<&McpManager>,\n) {\n    \
             if let Some(mcp) = mcp {\n        \
             defs.extend_from_slice(mcp.tool_definitions());\n    }\n    \
             seed_mcp_data_grades(grades, mcp);\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, helper);
    assert!(
        lectures.is_empty(),
        "le corps du helper est le seul site où lire pour publier est juste"
    );
    assert!(
        appels.is_empty(),
        "et sa définition n'est pas un appel — la compter rapprocherait le compte \
             de deux avec un seul appel réel"
    );

    // 3b. Les deux lecteurs déclarés sont couverts — chacun dans son
    //     fichier, et **pas** dans celui de l'autre : le droit est attaché
    //     à une ligne précise, pas accordé globalement.
    let collecte = ".flat_map(|m| m.tool_definitions().iter().map(|d| d.name.clone())),\n";
    let (lectures, _) =
        usages_des_definitions_mcp(std::path::Path::new("src/agent_loop/mod.rs"), collecte);
    assert_eq!(lectures.len(), 1);
    assert!(
        lecteur_declare("agent_loop/mod.rs", &lectures[0].1).is_some(),
        "la collecte de noms est un lecteur déclaré"
    );
    assert!(
        lecteur_declare("kg/domain_builder.rs", &lectures[0].1).is_none(),
        "mais son droit ne vaut que dans son fichier"
    );
    let projection = "for tool_def in mcp.tool_definitions() {\n";
    let (lectures, _) =
        usages_des_definitions_mcp(std::path::Path::new("src/kg/domain_builder.rs"), projection);
    assert_eq!(lectures.len(), 1);
    assert!(
        lecteur_declare("kg/domain_builder.rs", &lectures[0].1).is_some(),
        "la projection KG est un lecteur déclaré"
    );

    // 3c. Un appel réel au helper est compté.
    let appelant = "publier_outils_mcp(&mut defs, &mut grades, params.mcp_manager);\n";
    let (_, appels) = usages_des_definitions_mcp(ailleurs, appelant);
    assert_eq!(appels.len(), 1, "un appel réel doit être compté");

    // 3d. La prose qui nomme les aiguilles ne lit ni n'appelle rien — sans
    //     quoi les doc-comments de ce fichier, et les fixtures ci-dessus,
    //     rendraient la garde rouge (classe mika#2050 Signal S).
    let prose = "\
// Toute publication passe par `publier_outils_mcp(`, jamais par `mcp.tool_definitions()` nue.
/// `defs.extend_from_slice(mcp.tool_definitions())` est la forme d'avant mika#2814.
 * `publier_outils_mcp(` publie et ensemence dans le même appel.
";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, prose);
    assert!(
        lectures.is_empty() && appels.is_empty(),
        "un commentaire ne publie ni n'appelle — prédicat non ancré, classe \
             mika#2050 Signal S"
    );

    // 3e. Une région de test est masquée, et un fichier dont le chemin est
    //     du test n'a pas de moitié production : les deux exclusions qui
    //     rendent les fixtures de ce test écrivables dans ce fichier même.
    let dans_les_tests = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    \
             defs.extend_from_slice(mcp.tool_definitions());\n    \
             publier_outils_mcp(&mut d, &mut g, m);\n}\n";
    let (lectures, appels) = usages_des_definitions_mcp(ailleurs, dans_les_tests);
    assert!(
        lectures.is_empty() && appels.is_empty(),
        "mika#2398 — les régions de test sont masquées"
    );
    let chemin_test = std::path::Path::new("src/db/tests/harnais.rs");
    let (lectures, _) = usages_des_definitions_mcp(chemin_test, hors_helper);
    assert!(
        lectures.is_empty(),
        "mika#2321 — un fichier sous `tests/` est du test quel qu'en soit le contenu"
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

// ===========================================================================
// mika#1926 — a team child inherits the conversational parent's stop-topics
// ===========================================================================

const CHILD_AGENT: &str = "mika-1926-child";
const PARENT_AGENT: &str = "mika-1926-parent";

/// The system prompt of one real team-child turn, seeded with the child's own
/// and the parent's `stop_topic_*` rows.
///
/// End-to-end by construction (AC3): it goes through `run_team_agent`, so a
/// regression in the field wiring, in the merge, or in the `<stopped-topics>`
/// rendering reddens it — none of which a unit test on the merge alone could
/// see.
///
/// **The two agents share ONE handle, re-scoped with `with_agent`.** There is
/// one SQLite file per container, so the parent read is a different
/// `agent_id` on the same file. Two `Database::open_in_memory` calls would be
/// two distinct files: the parent's rows would be unreachable and the test
/// would pass for `parent: None` and fail for everything else, for a reason
/// that has nothing to do with the behaviour under test.
async fn team_turn_system_prompt(
    parent: Option<&str>,
    child_prefs: &[(&str, &str)],
    parent_prefs: &[(&str, &str)],
) -> String {
    use mika_common::llm::mock::{MockLlmProvider, text_response};

    let db =
        AsyncDatabase::new_with_agent(crate::db::Database::open_in_memory().unwrap(), CHILD_AGENT);
    // `preferences.agent_id` is an FK onto `agents(id)` — both ends must exist
    // before either can carry a row.
    db.register_agent(CHILD_AGENT, "child", "").await.unwrap();
    db.register_agent(PARENT_AGENT, "parent", "").await.unwrap();
    for (category, value) in child_prefs {
        db.set_preference(category, value).await.unwrap();
    }
    let parent_db = db.with_agent(PARENT_AGENT);
    for (category, value) in parent_prefs {
        parent_db.set_preference(category, value).await.unwrap();
    }

    let session_id = "team-mika1926-child";
    db.create_session_if_not_exists(session_id, CHILD_AGENT, "team", None)
        .await
        .unwrap();

    // Three identical responses rather than one: an EndTurn post-condition
    // guard may force a correction turn, and an exhausted mock sequence
    // panics. The assertion reads `captured_requests()[0]` either way.
    let mock = MockLlmProvider::builder()
        .response(text_response("travail fait"))
        .response(text_response("travail fait"))
        .response(text_response("travail fait"))
        .build();
    let tools = crate::tools::ToolRegistry::new();
    let skills = SkillRegistry::empty();
    let home_dir = tempfile::tempdir().unwrap();
    let skills_dirty = AtomicBool::new(false);
    let params = TeamAgentParams {
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        db: &db,
        llm: &mock,
        tools: &tools,
        skills: &skills,
        home_dir: home_dir.path(),
        task_message: "rédige la section",
        team_context: "Tu fais partie d'une équipe.",
        session_id,
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        gateway_url: None,
        internal_token: None,
        github_app: None,
        skills_dirty: &skills_dirty,
        settings: None,
        mcp_manager: None,
        agent_name: "child",
        child_task_id: None,
        message_sender: None,
        trace_id: None,
        parent_agent_id: parent,
    };
    run_team_agent(&params).await.expect("team turn");
    mock.captured_requests()
        .first()
        .expect("the turn must have reached the provider")
        .system
        .clone()
        .expect("a team turn always carries a system prompt")
}

/// W1 — the decision itself: a stop the user gave the parent reaches the
/// child's prompt.
#[tokio::test]
async fn mika1926_un_stop_du_parent_atteint_le_prompt_de_lenfant() {
    let system = team_turn_system_prompt(
        Some(PARENT_AGENT),
        &[],
        &[("stop_topic_vacances", "ne me reparle pas des vacances")],
    )
    .await;

    assert!(
        system.contains("<stopped-topics>"),
        "no <stopped-topics> block: the parent's row did not reach the child.\n{system}"
    );
    assert!(
        system.contains("vacances"),
        "the block is there but the parent's topic is not in it.\n{system}"
    );
}

/// W2 — the negative control, without which W1 proves nothing: the block
/// must be ABSENT when no parent is declared. A test that only ever sees the
/// positive case cannot tell inheritance from unconditional rendering.
#[tokio::test]
async fn mika1926_sans_parent_declare_lenfant_nherite_de_rien() {
    let system = team_turn_system_prompt(
        None,
        &[],
        &[("stop_topic_vacances", "ne me reparle pas des vacances")],
    )
    .await;

    assert!(
        !system.contains("<stopped-topics>"),
        "the block rendered without a declared parent.\n{system}"
    );
    assert!(
        !system.contains("vacances"),
        "the parent's topic leaked without a declared parent.\n{system}"
    );
}

/// W3 + W4 — the merge's three properties in one table: NOCASE dedup, the
/// child winning a collision, the case-insensitive order, and the
/// parent == child short-circuit.
#[tokio::test]
async fn mika1926_lunion_dedoublonne_en_nocase_et_lenfant_gagne() {
    // `preferences.category` is COLLATE NOCASE with PRIMARY KEY
    // (agent_id, category): `stop_topic_Web-Config` and
    // `stop_topic_web-config` are ONE topic, declared on both sides with
    // conflicting values.
    let system = team_turn_system_prompt(
        Some(PARENT_AGENT),
        &[
            ("stop_topic_Web-Config", "enfant"),
            ("stop_topic_zebra", "enfant-zebra"),
        ],
        &[
            ("stop_topic_web-config", "parent"),
            ("stop_topic_alpha", "parent-alpha"),
        ],
    )
    .await;

    let block = system
        .split_once("<stopped-topics>")
        .expect("block present")
        .1
        .split_once("</stopped-topics>")
        .expect("block closed")
        .0;
    assert_eq!(
        block.lines().filter(|l| l.contains("stop_topic")).count(),
        3,
        "three topics, not four — the collision must collapse.\n{block}"
    );
    assert!(
        block.contains("enfant") && !block.contains("parent\n"),
        "the child's value must win the collision.\n{block}"
    );
    let (alpha, web, zebra) = (
        block.find("alpha").expect("alpha"),
        block.find("Web-Config").expect("Web-Config"),
        block.find("zebra").expect("zebra"),
    );
    assert!(
        alpha < web && web < zebra,
        "order must be case-insensitive on `category`: alpha < Web-Config < zebra.\n{block}"
    );

    // W4 — the conversational agent can run as a member of its own team.
    // Re-reading its own scope would double every row, so the short-circuit
    // must leave exactly one line per topic.
    let self_parent = team_turn_system_prompt(
        Some(CHILD_AGENT),
        &[("stop_topic_zebra", "enfant-zebra")],
        &[],
    )
    .await;
    assert_eq!(
        self_parent.matches("stop_topic_zebra").count(),
        1,
        "parent == child must not duplicate the row.\n{self_parent}"
    );
}
