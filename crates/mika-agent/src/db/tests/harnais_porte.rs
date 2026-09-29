//! mika#2310 — isolated harness for the mika#1620 / mika#2287 gate, predicate
//! level. Module path: `db::tests::harnais_porte`, declared as an ordinary
//! sibling from `db/tests/mod.rs`.
//!
//! The campaign (mika#2288) used live grooms as the gate's primary proof; this
//! submodule replaces that, so a live run only exercises the layers AROUND the
//! gate (budgets, provider, worktree). mika#2287 already shipped seven of the
//! nine cases the spec enumerates — the three that remained are here (4 and 8)
//! and in `skills::executor::tests::harnais_porte` (9, 9b).
//!
//! The name is the campaign's, not the mechanism's: it is what the ticket's
//! exit criterion interrogates (`cargo test -p mika-agent harnais_porte`) and
//! what mika#2288 will read it by.
//!
//! **Why a separate file, then and now.** mika#2310 extracted it because `db.rs`
//! sat 4.6 KB under the 1 MB cap `scripts/check-secrets.sh` enforces in the
//! pre-commit hook and in CI, so any test added inline crossed it — and neither
//! weakening the guard nor bypassing the hook was that ticket's to do. It cost
//! an allowlist entry all the same, and that entry was not a static exemption
//! but an unbounded one: `db.rs` went on to take ~64 KB in silence, which is
//! the debt mika#2321 came back to settle.
//!
//! What changed with mika#2321 is only *how it is reached*. The
//! `#[path = "harnais_porte.rs"]` existed because `mod tests` was an **inline**
//! module of `db.rs`, so natural resolution would have looked under
//! `db/tests/tests/`. Now that `tests` is itself a file module under
//! `db/tests/`, this file is an ordinary sibling and the attribute is gone. The
//! module path — the thing the exit criterion actually interrogates — is
//! unchanged, and so is `cargo test -p mika-agent harnais_porte`.

use super::*;
use crate::task_state::tasks::{GROOM_REJECTED_JSON_ENVELOPE, GroomConvergence};

/// Case 4 — the only term of the predicate a live run ever refuted.
///
/// Trials 3/4 of mika#2288 failed on a callback that never reached
/// `completed`. The predicate filters `status IN ('completed',
/// 'delivered')` and the only non-terminal status covered before this
/// was `pending` (25744) — which carries NO `result` at all, so it is
/// also excluded by `instr(child.result, marker) > 0` and proves nothing
/// about the status filter. A `failed` row built this way carries the
/// marker AND the status, so the status is the single term separating it
/// from the nominal case.
///
/// Built entirely through the production write API, per condition 5 of
/// the mika#2287 GO ("zero raw SQL INSERT in tests"):
/// `completed_groom_pair` writes the result via `update_task_completed`,
/// then `update_task_status` (`db.rs:6552`, `pub`, no transition guard,
/// does not touch `result`) moves the status. The precedent is in this
/// same file — `test_groom_cross_check_delivered_callback_returns_true`
/// (25696) is this exact gesture with `delivered`.
#[test]
fn harnais_porte_cas4_failed_callback_returns_false() {
    let db = db();
    let (_, callback_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    db.update_task_status(&callback_id, "failed").unwrap();
    // `Absent` et non `!is_converged()` : la row est écartée par le filtre de
    // statut, donc rien n'est lu et rien n'est écarté au sens de R6. Un
    // `MarkerOutOfPosition` ici dirait que la ligne a été lue puis refusée pour
    // sa forme — deux diagnostics opposés que le booléen confondrait.
    assert_eq!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
        GroomConvergence::Absent,
        "a groom callback that carries `Outcome: PLAN_GROOMED` but died \
         `failed` is not proof of grooming — dropping the \
         `status IN ('completed','delivered')` filter would make a dead \
         groom count as proof (mika#2288 trials 3/4)"
    );
}

/// Case 4, twin — `cancelled` rather than `failed`. Two distinct tests
/// and not one parameterised: the operator counts the two populations
/// separately (a cancel is a decision, a failure is an accident), and a
/// parameterised test that regressed would not say which one moved.
#[test]
fn harnais_porte_cas4_cancelled_callback_returns_false() {
    let db = db();
    let (_, callback_id) =
        completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    db.update_task_status(&callback_id, "cancelled").unwrap();
    assert_eq!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
        GroomConvergence::Absent,
        "a groom callback cancelled after completing is not proof of grooming"
    );
}

/// **mika#2590 R10 — la note d'un refus n'est pas une preuve.**
///
/// Le défaut mesuré le 2026-09-29 sur mika#2105 : le callback de groom
/// `89165fb4` porte le JSON d'auto-skip de `dispatch-lib.sh`, dont le champ
/// `note` **cite le marqueur en toutes lettres** pour expliquer qu'aucune preuve
/// n'est frappée. `instr(result, 'Outcome: PLAN_GROOMED')` rend 651 : le texte
/// qui dit « ceci n'est pas une preuve » **est** la preuve. La porte a laissé
/// partir un pilote *implement* sur un ticket jamais re-groomé.
///
/// Deux raisons de refuser, et une seule suffirait — la ligne est refusée parce
/// que le `result` est une **enveloppe JSON portant `status`** (R2), et elle le
/// serait aussi parce que le marqueur n'y est **pas en position de verdict**
/// (R3). Le motif rendu est le premier : une enveloppe de saut n'est pas un
/// texte de callback mal formé, et distinguer les deux populations est ce que
/// [`crate::task_state::tasks::ALL_GROOM_CONVERGENCE_REJECTIONS`] existe pour
/// permettre.
///
/// Construit **entièrement par l'API d'écriture de production**
/// (`completed_groom_pair` → `create_task` + `update_task_completed`), condition
/// 5 du GO mika#2287 : zéro `INSERT` brut.
#[test]
fn mika2590_un_auto_skip_nest_pas_une_preuve_de_grooming() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_AUTO_SKIPPED);
    assert_eq!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
        GroomConvergence::MarkerOutOfPosition(GROOM_REJECTED_JSON_ENVELOPE),
        "le JSON d'auto-skip de `dispatch-lib.sh` cite `Outcome: PLAN_GROOMED` \
         dans sa prose pour dire qu'aucune preuve n'est frappée ; une lecture par \
         sous-chaîne en fait une preuve et laisse partir un implement sans plan \
         re-mesuré (mika#2590, mesuré sur mika#2105 le 2026-09-29)"
    );
}

/// Le contrôle **positif** de son voisin ci-dessus.
///
/// Sans lui, « l'auto-skip est refusé » serait indistinguable de « plus rien
/// n'est jamais accepté » — un prédicat qui refuse tout satisferait le test
/// négatif en entier tout en cassant la boucle.
#[test]
fn mika2590_une_ligne_ancree_reste_une_preuve() {
    let db = db();
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
    assert_eq!(
        db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
        GroomConvergence::Converged,
        "un groom réellement convergé porte le marqueur en début de ligne et \
         doit rester une preuve — c'est le faux négatif de KTD3 que ce contrôle \
         refuse"
    );
}

/// Case 8 — the DB reader really produces `Err`, it does not answer
/// `Ok(false)`.
///
/// `executor.rs`'s `test_groom_provenance_verdict_db_error_fails_closed`
/// (8092) builds `Err(anyhow!(…))` by hand and checks the verdict refuses
/// it. That is the useful half of the contract; the other half — *the
/// reader propagates an error rather than reporting absence* — was
/// attested by nothing. The distinction is not academic: `Ok(false)` and
/// `Err` produce two different rejections (`dispatch_grooming_not_verified`
/// vs `dispatch_check_failed`), and one tells the operator "this ticket
/// was never groomed" where the other says "I could not read my proof".
/// Confounding them sends the operator to re-groom a ticket whose
/// database is down.
///
/// **Substitution, stated because the test name cannot say it:** the
/// ticket words case 8 as "DB closed/corrupted". Neither is
/// constructible — `Database` does not expose closing its connection
/// without consuming itself, and corruption needs a dirtied file, hence
/// non-deterministic I/O. "Table absent" is the nearest deterministic
/// neighbour; the three raise `rusqlite::Error` by distinct mechanisms,
/// so this attests a neighbour of the stated case. Recorded in the body
/// of mika#2310 (AC9), not only here.
///
/// **This `DROP TABLE` is staging of the failure, not fabrication of
/// proof.** Condition 5 of the mika#2287 GO forbids building by SQL the
/// state the predicate must read; here the SQL destroys the read
/// support and writes no row the predicate would count. Without this
/// paragraph the next reader takes it as a precedent for raw writes.
///
/// The assertion is on the variant, never on the message — the wording
/// of "no such table" belongs to SQLite, not to us.
#[test]
fn harnais_porte_cas8_unreadable_predicate_returns_err_not_ok_false() {
    let db = db();
    db.conn
        .execute("DROP TABLE tasks", [])
        .expect("staging the failure must itself succeed");
    let outcome = db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL);
    assert!(
        outcome.is_err(),
        "an unreadable predicate must propagate `Err` so the caller \
         answers `dispatch_check_failed`; `Ok(false)` would invert the \
         fail-closed contract and report \"not groomed\" for a broken \
         database (mika#2287 GO condition 2, other side)"
    );
}
