---
module: crates/mika-agent/src/server/ready_label_handler, skills/bundled/_shared/dispatch-lib
tags: [autonomous-loop, engine-side-dispatch, dev-groom, dev-pilot, policy-deny, dispatch-lib, worktree-setup, ready-label, silent-failure]
problem_type: bug
category: workflow-issues
date: 2026-06-27
ticket: mika#1593
related: mika#1572 (regressing change), mika-platform#58 (branch-name centralization), mika#1318 (pilot-no-push)
applies_when:
  - A ready-labelled ticket dispatches dev-groom/dev-pilot but the inner pilot hits a tier-1 policy-deny on scripts/derive-branch-name
  - Changing what dispatch `prompt` string the engine-side ready-label handler emits
  - Changing dispatch-lib's _set_up_worktree repo#number parser
  - Debugging "the pilot ran but no worktree was set up" symptoms
resolution_type: code_fix
---

# Engine-side dispatch emitted an owner-qualified prompt that dispatch-lib silently dropped to free-text mode

## TL;DR

mika#1572 (engine-side ready-label dispatch) built the dispatch `prompt` as the
**owner-qualified** `senara-solutions/mika#N` via `ReadyLabelLocation::owner_repo()`.
dispatch-lib's `_set_up_worktree` only accepts the **bare** `mika#N` form
(regex `^[a-zA-Z0-9_-]+#[0-9]+$` — no slash). The slash failed the parse, `REPO`
stayed empty, and the dispatch silently fell through to the `else` "free-text mode,
no worktree" branch. claude-pilot then launched in the meta-repo root with **no
worktree**, the inner session improvised worktree setup by calling
`scripts/derive-branch-name` directly, and claude-pilot-py's tier-1 classifier
denied it. Every engine-side dev-groom/dev-pilot dispatch wedged (tickets #1591,
#1576, #1573 on 2026-06-27).

## The misleading symptom

The visible failure is identical to a genuine drift/policy case:
`[policy:deny]` on `scripts/derive-branch-name`, then `PIPELINE FAILURE`. The
ticket's own stated root cause ("the engine handler skips the worktree-setup
phase") was **wrong** — the handler *does* run the same
`handlers/run.sh → dispatch-lib.sh → _set_up_worktree` path the LLM-tool dispatch
used. The defect was one layer deeper: a **prompt-format contract mismatch** that
made worktree-setup a silent no-op. The symptom (`derive-branch-name` deny) is the
*downstream consequence* of the inner pilot improvising, not the cause. Compare
the same symptom from a different cause in
`2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md`.

## Why it was silent (and expensive)

The owner-qualified prompt did not *error* — it matched a legitimate code path
(`_set_up_worktree`'s free-text `else` branch, `CWD_ARGS="--cwd $PLATFORM_DIR"`)
that is correct for genuine free-text dispatch but wrong for a ticket ref. So
worktree-setup "succeeded" by doing nothing, and the failure only surfaced ~7
pilot turns and $0.20 later when the inner session hit the classifier. A loud
parse error would have been caught at dispatch.

## The contract

The dispatch `prompt` argument is **bare `repo#number`** (`mika#214`,
`mika-skills#8`) — documented in `dev-pilot/tools.json` and `dev-groom/tools.json`,
required by dispatch-lib's parser, and what the historical LLM-tool path passed.
`owner_repo()` (owner-qualified) is for `gh` calls, task labels, audit, and the
cosmetic `[GitHub] Issue labeled ready on …` marker line — **never** for a
dispatch prompt.

## The fix (two layers)

1. **Producer (primary):** `ReadyLabelLocation::repo_name()` returns the bare
   basename (strip any `owner/`). Both dispatch-prompt emitters — the engine-side
   `dispatch_input` and the #1571 fallback pre-digest `format_ready_label_pre_digest`
   — now use it. The cosmetic marker line stays owner-qualified.
2. **Consumer (defense-in-depth):** dispatch-lib's parser regex broadened to
   `^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$` and normalizes `REPO` to the
   basename (`sed 's/#.*//' | sed 's#.*/##'`). An owner-qualified ref now routes
   into worktree mode instead of silently falling through. The match stays fully
   anchored, so genuine free-text prompts (with spaces / embedded `#`) still fall
   through correctly.

Both layers preserve mika-platform#58 (dispatch-lib's `derive-branch-name` script
stays the sole brancher — no LLM-improvised names) and mika#1318 (pilot-no-push,
untouched).

## Lessons

- **A dispatch-prompt string is a parsed contract, not display text.** When the
  engine simulates "what the LLM would have provided," it must emit the exact form
  the consumer's parser documents — here, the bare form the tool schemas specify,
  not the owner-qualified form convenient for `gh`/display.
- **Silent fall-through to a valid-but-wrong branch is the dangerous failure
  mode.** A parser that has both a strict-match branch and a permissive
  catch-all (free-text mode) will hide a format mismatch as a no-op. Either
  normalize tolerantly *or* fail loud — never silently route a near-miss into the
  catch-all.
- **Verify the stated root cause against the code before implementing.** The
  ticket's "handler skips worktree-setup" framing would have produced a fix that
  fixed nothing (worktree-setup already runs). The real cause was visible only by
  reading the parser regex against what the handler actually emits.
