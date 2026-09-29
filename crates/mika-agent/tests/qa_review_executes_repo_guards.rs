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

// ---------------------------------------------------------------------------
// mika#2565 — Layer C. A Rust bump is verified by compiling, never by asserting.
//
// These scans read the prompt as text and prove nothing about what the model
// does with it; the behavioural half is the calibration scenario
// `dependabot_cargo_bump_requires_build` and sonde S1. What they close is the
// regression class: the unconditional build skip coming back, or Step 3e being
// re-forbidden on the one path that now needs it.
// ---------------------------------------------------------------------------

/// **The prescription that produced the defect must be gone.**
///
/// Step 1.6 item 2 read, literally: "Emit … `BUILD VERIFICATION: skipped
/// (Dependabot dependency PR)`". Unconditional. That is what the four measured
/// `pass` verdicts were obeying — the `(pipeline-exempt label)` wording they
/// carried was the model's own paraphrase, and the label exempts the plan, not
/// the build (R1 of the plan).
#[test]
fn mika2565_step_1_6_no_longer_prescribes_an_unconditional_build_skip() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        !prompt.contains("`BUILD VERIFICATION: skipped (Dependabot dependency PR)`"),
        "qa-review/system_prompt.md Step 1.6 prescribes an UNCONDITIONAL build \
         skip again (mika#2565).\n\
         That exact string is what mika#2560 and mika#2561 were obeying when \
         they were approved with a red required check behind them. The build \
         skip must be conditioned on the diff: a `Cargo.toml` / `Cargo.lock` \
         path makes the build REQUIRED (step 5c); anything else skips with a \
         reason that is true — `(dependency PR, no Rust dependency resolution \
         in the diff)`."
    );
}

/// **The label must not be offered as a build dispensation.**
///
/// This is the topical stop rather than an enumeration of forbidden phrases:
/// the prompt does not list "API-compatible" / "API-surface verified" — listing
/// them would hand the model the template it is being denied (mika#2292), and
/// R2 established the phrase was never the problem. What is pinned is narrower
/// and checkable: the label and the build must not be tied together.
#[test]
fn mika2565_step_1_6_does_not_present_the_label_as_a_build_exemption() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        !prompt.contains("skipped (pipeline-exempt label)"),
        "qa-review/system_prompt.md offers `pipeline-exempt` as a build \
         exemption (mika#2565).\n\
         It exempts the PLAN and has never exempted the BUILD. The measured \
         verdicts invoked it in exactly this wording; the prompt must not make \
         that reading available."
    );
}

/// **Step 3e is no longer forbidden on the one path that now needs it.**
///
/// The closing line of Step 1.6 read "Do NOT run Steps 2/2.5/3e for a
/// Dependabot PR". Step 5c *is* this path's build verification, so leaving 3e
/// in that list would make the new step contradict its own section.
#[test]
fn mika2565_step_1_6_no_longer_forbids_step_3e() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    assert!(
        !prompt.contains("Do NOT run Steps 2/2.5/3e"),
        "qa-review/system_prompt.md Step 1.6 forbids Step 3e again (mika#2565 \
         AC5).\n\
         Step 5c is this path's build verification and reuses 3e's worktree \
         formula and its end-the-turn discipline. Forbidding 3e here makes the \
         section contradict itself, and the build never runs."
    );
    assert!(
        prompt.contains("Do NOT run Steps 2/2.5 for a Dependabot PR"),
        "the plan-AC skip must still be prescribed — this scan removes `3e` \
         from that list, it does not remove the list."
    );
}

/// **The build step must actually be reachable and executable as written.**
///
/// A prompt that said "verify the build" without naming the tool, or without
/// the worktree the tool needs, would be the inert branch the plan's O1 names:
/// Step 3e's own path derivation ends in "skipped (no worktree found at
/// expected path)" for a Dependabot PR, because the loop never dispatched one.
#[test]
fn mika2565_step_1_6_names_the_build_tool_and_creates_its_worktree() {
    let prompt = qa_review_prompt();
    assert_step_1_6_exists(&prompt);

    let step_5c = prompt
        .split_once("5c. **Compile it")
        .expect(
            "qa-review/system_prompt.md Step 1.6 no longer carries step 5c \
             (mika#2565). Every scan below targets it; establish where the \
             build requirement lives now BEFORE touching these tests.",
        )
        .1
        .split_once("\n8. **Verdict mapping")
        .expect("step 5c must sit before item 8, which consumes its result")
        .0;

    assert!(
        step_5c.contains("build_mika(cwd="),
        "step 5c must name the build tool and its argument, or it prescribes an \
         intention rather than a gesture"
    );
    // The `worktree add` lines, not the prose around them. The section
    // legitimately *mentions* `--detach` to say it is not used, and a bare
    // `contains` over the whole section would read that mention as the gesture
    // — the false-positive class mika#2050 measured on Signal S. Every line
    // that carries the subcommand is checked rather than the first one, so a
    // second invocation added later cannot slip past under the first's cover.
    let creations: Vec<&str> = step_5c
        .lines()
        .filter(|l| l.contains("worktree add"))
        .collect();

    assert!(
        !creations.is_empty(),
        "step 5c must CREATE the worktree (plan O1): a Dependabot PR is never \
         dispatched by the loop, so the path Step 3e derives does not exist and \
         the build would skip under one more name"
    );
    assert!(
        creations.iter().any(|l| l.contains(".claude/worktrees/")),
        "the worktree must be created under the MANAGED root (plan O2) — that \
         is term T1 of the mika#2420 reaper and the population of the mika#2497 \
         `target/` purge. Elsewhere it is a 15-50 GB orphan nothing reaps.\n\
         lines read: {creations:?}"
    );
    assert!(
        !creations.iter().any(|l| l.contains("--detach")),
        "the worktree must be ATTACHED to the branch (plan O2), so the reaper \
         resolves it by branch key (T3) rather than through mika#2518's SHA \
         catch-up.\n\
         lines read: {creations:?}"
    );
    assert!(
        step_5c.contains("END YOUR TURN"),
        "step 5c must end the turn after the build call — `build_mika` is \
         long-running and any verdict posted before its callback is premature"
    );
}

// ---------------------------------------------------------------------------
// mika#2565 — the build callback
// ---------------------------------------------------------------------------

fn qa_review_build_callback_prompt() -> String {
    let path = workspace_root().join("skills/bundled/qa-review-build-callback/system_prompt.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// **The callback's dependabot branch must come BEFORE the plan re-read.**
///
/// The callback opens on "Mandatory plan re-read … If the plan file is
/// unreadable: `VERDICT: block[pipeline]` … NOT downgraded to `hold[review]`".
/// A Dependabot PR has no plan, so on that class the callback would produce
/// exactly the verdict mika#2519's B1 refuses structurally — the review would
/// die on a tool refusal rather than on a judgment (plan O3).
///
/// Order is the whole assertion: a branch placed after the re-read is a branch
/// the re-read has already pre-empted.
#[test]
fn mika2565_the_build_callback_branches_on_dependabot_before_the_plan_reread() {
    let prompt = qa_review_build_callback_prompt();

    let dependabot_branch = prompt
        .find("0. **Dependabot dependency PR — there is no plan to re-read")
        .expect(
            "qa-review-build-callback/system_prompt.md no longer carries its \
             Dependabot branch (mika#2565 / plan O3). Without it the callback \
             emits `block[pipeline]` on a plan-less PR — the verdict mika#2519 \
             B1 refuses before the subprocess, so the review dies on a tool \
             refusal.",
        );
    let plan_reread = prompt
        .find("**Mandatory plan re-read")
        .expect("the plan re-read must still exist — this scan orders it, it does not remove it");

    assert!(
        dependabot_branch < plan_reread,
        "the Dependabot branch must come BEFORE the mandatory plan re-read \
         (mika#2565). Placed after it, the re-read has already run and already \
         emitted `block[pipeline]` for a plan that was never supposed to exist."
    );

    assert!(
        prompt.contains("Never emit `block[pipeline]` on this class"),
        "the branch must say what it forbids, not merely where to resume: \
         `block[pipeline]` is structurally unreachable here (mika#2519 B1) and \
         the engine refuses the call before the subprocess"
    );
    assert!(
        prompt.contains("no `> - **Plan:** \\`<path>\\`** callout")
            || prompt.contains("no `> - **Plan:**"),
        "the discriminant must be the ABSENCE OF THE PLAN CALLOUT — the fact \
         item 1 already derives `<plan-path>` from — and not the \
         `dependabot/` branch prefix, which would be a naming heuristic where a \
         fact is available"
    );
}
