//! Structural guard for mika#2172 — `qa-review` must EXECUTE the target repo's
//! pipeline guards, never re-implement them in prose.
//!
//! ## The defect this pins
//!
//! `qa-review/system_prompt.md` used to paraphrase `scripts/verify-pipeline.sh`.
//! The paraphrase drifted in both directions, and each direction cost a blocked
//! PR the day it mattered:
//!
//! - **A rule the script had removed on purpose.** Step 2 check 1 demanded a
//!   `docs/plans/*.md` in the diff. `verify-pipeline.sh` lines 71–78 say in so
//!   many words that this check was deleted because it rejected legitimate
//!   `/ce:compound` docs-only PRs. mika#2167 (21/21 green) was blocked on it.
//! - **A vocabulary belonging to one repo, applied to another.** The prompt
//!   hard-coded `(docs-only|code-only)` — `mika`'s vocabulary — and declared
//!   itself a verbatim mirror. `mika-platform`'s `plan-doc-check.sh` accepts
//!   `no-plan` and nothing else, so mika-platform#203 (7/7 green) had a trailer
//!   its own CI had just validated declared invalid.
//! - **A rule with no original at all.** The "tactical-surface auto-detect"
//!   auto-exempted `scripts/`, `os/`, `Dockerfile.`, `skills/bundled/_shared/`.
//!   No script carries that rule, and those paths ARE in `verify-pipeline.sh`'s
//!   `SOURCE_BUCKET` — so this one made qa PASS where CI blocks.
//!
//! ## Why a test and not a sentence
//!
//! The prompt already contained the sentence. Line 190 read "mirrors
//! `verify-pipeline.sh` lines 158–172 verbatim" while being, at that moment,
//! not a mirror at all. A copy that declares itself faithful is precisely the
//! one nobody re-checks — the same shape as mika#2158's two grooming
//! predicates, and closed the same way: by a check that fails when the copy
//! comes back.
//!
//! ## Boundary — what this test does NOT claim
//!
//! It reads the prompt as text. It cannot prove the LLM executes the guard, nor
//! that the guard's verdict is honored; those are behavioral and live with the
//! mika-qa calibration suite (`make calibrate-mika-qa`) and the ACs of
//! mika#2172. What it does close is the *regression* class: a future edit
//! re-introducing a hard-coded exemption vocabulary or a path-prefix allowlist
//! fails here, in CI, instead of on the next green PR it blocks.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

fn qa_review_prompt() -> String {
    let path = workspace_root().join("skills/bundled/qa-review/system_prompt.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// The two guard paths the prompt must discover. These are file paths, not
/// rules — a list of files can only ever drift in *coverage* (a repo ships a
/// guard under a third name), which the prompt reports as
/// `PIPELINE: not-applicable` rather than inventing a verdict for. That is a
/// strictly weaker drift class than the one mika#2172 closed.
const REQUIRED_GUARD_PATHS: &[&str] = &["scripts/verify-pipeline.sh", "scripts/plan-doc-check.sh"];

#[test]
fn prompt_names_both_repo_guard_paths() {
    let prompt = qa_review_prompt();
    for path in REQUIRED_GUARD_PATHS {
        assert!(
            prompt.contains(path),
            "qa-review/system_prompt.md no longer names the guard `{path}`.\n\
             Step 2 discovers guards by path existence; dropping one silently \
             narrows coverage on the repo that ships it (mika#2172 D1)."
        );
    }
}

#[test]
fn prompt_does_not_reimplement_the_exemption_vocabulary() {
    let prompt = qa_review_prompt();

    // The extraction regex, not the token. `Pipeline-Exempt:` still appears in
    // the prompt as prose ("`mika` accepts docs-only / code-only") and inside a
    // guard's own quoted output — both are descriptions of what the SCRIPT
    // does, which is what this ticket wants. What must not come back is the
    // prompt matching the trailer itself and ruling on it.
    let forbidden = "(docs-only|code-only)";
    assert!(
        !prompt.contains(forbidden),
        "qa-review/system_prompt.md carries the alternation `{forbidden}` again.\n\
         That is `mika`'s exemption vocabulary hard-coded into a prompt that \
         reviews every repo. `mika-platform`'s guard accepts `no-plan` and \
         nothing else — mika-platform#203 was blocked on exactly this, with a \
         trailer its own CI had validated green.\n\
         Let the repo's guard read its own trailer (mika#2172 AC2)."
    );
}

#[test]
fn prompt_does_not_carry_a_path_prefix_auto_exemption() {
    let prompt = qa_review_prompt();

    // The "tactical-surface auto-detect" allowlist. This one is the dangerous
    // direction: no script carries the rule, and every path in it is inside
    // `verify-pipeline.sh`'s SOURCE_BUCKET, so it made qa approve PRs that CI
    // rejects. Asserted on two independent fragments so a reordering of the
    // alternation does not slip through.
    for fragment in ["tactical-surface", "skills/bundled/_shared/|os/|scripts/"] {
        assert!(
            !prompt.contains(fragment),
            "qa-review/system_prompt.md carries `{fragment}` again — the \
             path-prefix auto-exemption (mika#2172 divergence 4).\n\
             No repo guard carries that rule, and `scripts/`, `os/`, \
             `Dockerfile.` and `skills/bundled/_shared/` are all in \
             `verify-pipeline.sh`'s SOURCE_BUCKET. Re-adding it makes qa \
             approve what CI blocks."
        );
    }
}

#[test]
fn prompt_does_not_reimplement_the_plan_doc_presence_check() {
    let prompt = qa_review_prompt();

    // The two blocking reasons Step 2 emitted for rules no `mika` script
    // carries. Matched as the reason strings rather than as greps, because the
    // grep `^docs/plans/.*\.md$` is still legitimately used by Step 2.5.4 for
    // the implicit no-parallel-plan structural AC — a different question, and
    // one no guard answers.
    for reason in [
        "Missing plan document in docs/plans/",
        "No source changes beyond documentation",
    ] {
        assert!(
            !prompt.contains(reason),
            "qa-review/system_prompt.md emits `{reason}` again.\n\
             `verify-pipeline.sh` removed both the plan-doc-presence and the \
             compound-doc-presence checks on purpose (lines 71–78): they are \
             subsumed by the bucket logic and rejected legitimate docs-only \
             ships with no escape hatch. mika#2167 was blocked on the first of \
             them while its own CI was 21/21 green (mika#2172 AC3)."
        );
    }
}

/// mika#2419 — Step 2B's synthetic event must carry the PR author.
///
/// ## Why a structural guard and not a behavioural test
///
/// `scripts/verify-pipeline.sh` learned an automated-author exemption
/// (`.pull_request.user.login` against a two-login list). Its only qa-side
/// caller is the `run_shell` command Step 2B prescribes, which builds the
/// `GITHUB_EVENT_PATH` payload by hand. Before mika#2419 that payload carried
/// `number` and `labels` and nothing else — so the guard would have known a
/// rule its caller never gave it the means to apply (class mika#2205).
///
/// Removing the field again would make **no decision wrong**: every case of
/// `scripts/verify-pipeline-test.sh` (F1–F7 included) drives the script
/// directly and would stay green, while the exemption went inert in
/// production. That is precisely the class a source scan exists to hold.
#[test]
fn step_2b_synthetic_event_carries_the_pr_author() {
    let prompt = qa_review_prompt();

    // The payload itself. Asserted as the literal Step 2B prints, because the
    // guard reads `.pull_request.user.login` and nothing else — a payload that
    // nested the login anywhere else would parse and resolve to empty, i.e.
    // fail closed, i.e. reproduce the defect while looking correct.
    assert!(
        prompt.contains(r#""user":{"login":"<author>"}"#),
        "qa-review/system_prompt.md Step 2B no longer builds the synthetic \
         event with `\"user\":{{\"login\":\"<author>\"}}`.\n\
         `scripts/verify-pipeline.sh` reads `.pull_request.user.login` for its \
         automated-author exemption (mika#2419). Without the field the guard \
         resolves an empty login, grants no exemption, and every dependabot \
         Cargo.lock-only PR goes back to `block[pipeline]` — the mika#2415 \
         symptom — with the whole shell test suite still green."
    );

    // And the field must be taken off `qa_pr_view` rather than invented: it is
    // already in that handler's SAFE_FIELDS, which is what makes the payload
    // reproducible from live PR metadata.
    assert!(
        prompt.contains("`labels`, `author`, and `body` from Step 1's `qa_pr_view`"),
        "qa-review/system_prompt.md Step 2B no longer extracts `author` from \
         Step 1's `qa_pr_view` output (mika#2419). The synthetic event's \
         `user.login` has to come from the PR's real author; sourcing it \
         anywhere else makes the exemption unreproducible."
    );
}

#[test]
fn prompt_requires_the_guard_output_to_be_quoted_verbatim() {
    let prompt = qa_review_prompt();

    // AC6: a pipeline verdict that does not exhibit the output of the guard it
    // claims to reflect is the same fault, one layer up. The `PIPELINE:`
    // section is the surface that makes the claim checkable by a human.
    assert!(
        prompt.contains("PIPELINE: not-applicable"),
        "qa-review/system_prompt.md no longer defines `PIPELINE: not-applicable`.\n\
         A repo with no executable guard must be reported as such, never judged \
         on another repo's rules (mika#2172 AC1, second sentence)."
    );
    assert!(
        prompt.contains("verbatim"),
        "qa-review/system_prompt.md no longer requires the guard output to be \
         quoted verbatim (mika#2172 AC6)."
    );
}

// ---------------------------------------------------------------------------
// mika#2519 — Step 1.6's two corrections, and the anti-vacuity term that keeps
// these scans from reading a disappeared section as a clean tree.
// ---------------------------------------------------------------------------

/// Every scan below aims at Step 1.6. A scan that aims at a section which no
/// longer exists reads **exactly** like a scan on a clean tree (mika#2103 /
/// mika#2205), so each one calls this first.
fn assert_step_1_6_exists(prompt: &str) {
    assert!(
        prompt.contains("**Step 1.6 — Dependabot dependency-PR path"),
        "qa-review/system_prompt.md no longer carries Step 1.6 at all.\n\
         Every mika#2519 scan in this file targets that section; with the \
         section gone they would all pass while asserting nothing. Establish \
         where the Dependabot path lives now BEFORE touching these tests."
    );
}

/// `author` is an **object**, and Step 1.6 used to compare it to a string.
///
/// `qa_pr_view` exposes `author` in its `SAFE_FIELDS`, and
/// `gh pr view --json author` renders
/// `{"id":…,"is_bot":true,"login":"app/dependabot","name":""}`. Step 1.6 read
/// *"If `author == \"dependabot[bot]\"`"* — an object↔string comparison **on the
/// single discriminant of the whole Dependabot path**. A model that took it
/// literally never entered the dep-review flow, and the failure is silent: the
/// PR simply goes down the plan-AC path it has no plan for.
#[test]
fn mika2519_step_1_6_reads_the_author_login_and_not_the_author_object() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        prompt.contains("`author.login`"),
        "qa-review/system_prompt.md Step 1.6 no longer names `author.login` \
         (mika#2519).\n\
         `author` is an OBJECT — comparing it to a string never matches, so the \
         whole Dependabot path goes unreachable while every test stays green."
    );

    // Both renderings must stay named: `gh` gives one or the other depending on
    // the surface, and `AUTOMATED_PR_AUTHORS` carries both for that reason.
    for login in ["dependabot[bot]", "app/dependabot"] {
        assert!(
            prompt.contains(login),
            "qa-review/system_prompt.md Step 1.6 no longer names `{login}`.\n\
             `gh` renders the bot identity either way; dropping one makes the \
             prompt disagree with `evidence::guards::AUTOMATED_PR_AUTHORS` and \
             with `scripts/verify-pipeline.sh`."
        );
    }
}

/// Step 1.6 names the major-version jump as a discriminant, and names the line
/// the engine reads.
///
/// **The half that holds is the engine guard, not this clause** — per
/// `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, and
/// writing it the other way round would reproduce the very defect mika#2519
/// closes. What this scan protects is the *intent* half: without it a model told
/// by the engine to emit `API-SURFACE:` has never been told why, nor what to
/// read before asserting it.
#[test]
fn mika2519_step_1_6_names_the_major_jump_and_the_api_surface_line() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        prompt.contains("API-SURFACE:"),
        "qa-review/system_prompt.md Step 1.6 no longer names the `API-SURFACE:` \
         line (mika#2519).\n\
         That line is what the engine reads to let a `pass` through on a major \
         bump (`evidence::guards::body_asserts_api_surface`). Without the \
         prompt half the model is refused a verdict it was never told how to \
         ground — and the canonical-token row for it goes stale."
    );

    assert!(
        prompt.contains("first numeric segment"),
        "qa-review/system_prompt.md Step 1.6 no longer defines the major-jump \
         discriminant as the FIRST NUMERIC SEGMENT (mika#2519).\n\
         The engine's `classify_version_bump` uses exactly that rule, and \
         deliberately does NOT apply semver's `0.x` convention: a looser prompt \
         would ask the model to block `0.22 -> 0.23`, i.e. four of the five \
         measured witness PRs."
    );

    // The measured counter-example has to stay in the prompt: it is the whole
    // reason a green build is not the gate on this class (mika#2525).
    assert!(
        prompt.contains("jsonwebtoken"),
        "qa-review/system_prompt.md Step 1.6 no longer cites the measured case \
         (`jsonwebtoken 9.3.1 -> 11.1.0`, mika#2454 / mika#2525: build green, \
         `generate_jwt` panicking).\n\
         Without it the clause reads as a precaution rather than as a \
         measurement, and a model weighing it against a green build has no \
         reason to prefer the clause."
    );
}

/// The exemption is not written a third time — U5.
///
/// The plan exemption for dependency PRs exists at two places already
/// (`scripts/verify-pipeline.sh` mechanism 4, and Step 1.6's skip of Steps 2 /
/// 2.5). mika#2172 closed, on this very prompt, the class where a rule posted at
/// a third place drifts from the other two. So Step 1.6 must keep **pointing
/// at** the skip it already prescribes, and must not grow a second vocabulary
/// for it.
#[test]
fn mika2519_step_1_6_still_delegates_the_plan_exemption_it_already_had() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        prompt.contains("Skip Step 2 pipeline checks and Step 2.5 plan-AC verification"),
        "qa-review/system_prompt.md Step 1.6 no longer carries its own plan \
         exemption (mika#1729).\n\
         mika#2519 relies on that skip being the ONE prompt-side exemption: it \
         adds the engine half that holds it, and deliberately writes no third \
         copy (U5). If the skip moved, the engine's refusal body — which names \
         Step 1.6 by name — now points at nothing."
    );
}
