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
