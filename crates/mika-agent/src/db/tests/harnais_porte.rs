//! mika#2310 — isolated harness for the mika#1620 / mika#2287 gate, predicate
//! level. Module path: `db::tests::harnais_porte` (declared with `#[path]` from
//! `db.rs` → `mod tests`).
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
//! **Why a separate file and not an inline block in `db.rs`:** `db.rs` on main
//! weighs 1 043 979 bytes, 4.6 KB under the 1 MB cap that
//! `scripts/check-secrets.sh` enforces in the pre-commit hook and in CI. Any
//! test added inline crosses it. Neither weakening the guard (allowlisting a
//! source file) nor bypassing the hook is this ticket's to do; `#[path]` keeps
//! the module path the plan names (D3) while leaving `db.rs` under the cap.

use super::*;

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
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
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
    assert!(
        !db.has_completed_groom_for_issue("mika", GROOM_ISSUE_URL)
            .unwrap(),
        "a groom callback cancelled after completing is not proof of grooming"
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
