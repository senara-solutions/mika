---
title: "GitHub issue-comment events autonomously fire mika-dev → claude-pilot, with the comment body as the working context"
date: 2026-04-25
category: workflow-issues
module: self-dev
problem_type: workflow_issue
component: webhook_event_routing
severity: medium
applies_when:
  - Posting comments on open mika repo issues for any reason — review, status, capture, peer-feedback
  - Designing the contract surface that distinguishes "spec/plan/verdict for implementation" from "observation/discussion"
  - Reasoning about the cost / blast-radius of mika-dev's autonomous-dispatch envelope
tags:
  - mika-dev
  - claude-pilot
  - webhook-trigger
  - autonomous-dispatch
  - self-dev
  - issue-comment-event
  - mika-arch
  - architect-implementation-loop
  - ready-label
  - positive-consent
---

# GitHub issue-comment events autonomously fire mika-dev → claude-pilot

## Context

mika-dev subscribes to GitHub `issue_comment.created` webhook events. The handler skill family (`self-dev-webhook-qa`, `self-dev-webhook-ci`, and the broader `self-dev` umbrella) classifies incoming comments as actionable or non-actionable. **Actionable comments fire the full /mika pipeline autonomously**: mika-dev creates a task, marks it in-progress, and launches `claude-pilot` in a fresh worktree — all without a human "go" signal beyond posting the comment.

This is **by design**. The self-dev skill is built so that comments-as-implementation-spec is a first-class trigger surface alongside explicit `mika ask --agent mika-dev "implement <ref>"` dispatches. The intent is: a human (or an agent) posting a fully-specified plan on an open issue *is* the dispatch — it is the contract, and the autonomous loop honors it.

The 2026-04-25 mika-arch first dogfood was the first observed instance of the loop closing entirely autonomously from a GitHub comment, surprising the operator who posted the comment as a capture artifact rather than a dispatch.

## What happened

Sequence on 2026-04-25, ~20:13–20:14 UTC:

1. **Architect review.** mika-arch (newly born; first dogfood) reviewed the proposed plan for `senara-solutions/mika#814` (a config-loader bug surfaced during the post-deploy smoke test). Verdict: GROOMED. Session `f02d372e-3202-47fa-b095-2294fab147e4`.
2. **Operator captures the GROOMED plan** as a comment on `senara-solutions/mika#814`, intending it as the canonical implementation contract on the issue body — to be dispatched via `mika ask --agent mika-dev "implement mika issue#814"` next.
3. **GitHub fires `issue_comment.created`** webhook → mika-relay → mika-dev. Channel: `github`.
4. **mika-dev classifies the comment as actionable** and runs the dispatch chain in 5 tool calls over 8 LLM turns (~95s wall-clock):

   ```
   search_memory("mika#814")           → cold cache, no prior context
   create_task(label="MIKA_KG_DOCS_…") → task faabf2a5-… created, status=pending
   update_task_status(in_progress)     → task faabf2a5-… now in_progress
   run_claude_pilot(prompt="mika#814") → subprocess 5f1e29db-… spawned in worktree
   gh_read(issue_view, repo, 814)      → post-dispatch verification of issue state
   ```

5. **claude-pilot starts /mika** in worktree `/data/workspace/mika-platform/.claude/worktrees/fix-config-kg-docs-roots-env-list-parse/mika/`. Branch derivation honored the canonical `> - **Branch:** ` callout from the issue body (per the [grooming-branch-callout discipline](./grooming-branch-callout-required-2026-04-25.md) compounded earlier the same day) — branch `fix/config-kg-docs-roots-env-list-parse` matches the issue's specified branch verbatim.

6. **Operator notices** ~30s later when checking session state for an unrelated reason. Net surprise: an explicit `mika ask` was about to be issued; the autonomous loop got there first.

Total elapsed from comment post → claude-pilot running: under 60 seconds.

## Why this happened "right" but surprised the operator

The GROOMED plan I posted **looked exactly like an implementation spec** — because it was. The skill classifier had no reason to ignore it. From mika-dev's perspective:

- The comment was on an open `bug` ticket
- The comment body contained an "Approach" section with concrete code snippets
- The comment body contained "Files to touch", "Test scenarios", "Verification" sections
- The comment was authored by a known operator (samidarko)

That is the dispatch contract. The autonomous classifier doing what it's designed to do.

The surprise was not "wrong behavior" — it was **discovering that the architect → implementation loop closes entirely without an explicit dispatch step**. The GROOMED-plan comment functions as the dispatch. Once the architect approves a plan and someone posts it as a comment, autonomous implementation follows.

## Numbers (the autonomous dispatch turn)

| Metric | Value |
|---|---|
| Session | `e26d6bb9-922a-412e-89f7-f9eea4133ec8` (channel: `github`) |
| Task | `faabf2a5-a46e-476d-a9d6-7504807c663b` |
| claude-pilot subprocess | `5f1e29db-eb54-4e8e-95ff-aafbefb496e3` |
| LLM calls | 8 |
| Total input tokens | 312,217 |
| Total output tokens | 1,079 |
| Cache read tokens | 153,328 |
| Cache efficiency | 49% (cache_read / total_input) |
| Wall-clock to dispatch | ~95s |
| Model | `moonshotai/kimi-k2.5` (mika-dev's base) |

The cache efficiency is notably low for an 8-turn webhook flow. Steps 0, 3, 4, 6, 7 all missed cache entirely. Likely cause: the conversation prefix that mika-dev assembles for webhook-triggered turns isn't perfectly stable across turns — small variations (timestamp tokens, task-state injection) push the cache key. Worth a separate investigation; not blocking.

## Cost / blast-radius envelope

- **Per autonomous dispatch (mika-dev's decision turn):** ~95s, ~310K input tokens (Kimi K2.5 ≈ pennies). Negligible per-event.
- **Plus the claude-pilot subscription session:** the actual /mika pipeline run is a separate subprocess on the user's Anthropic subscription. Cost depends on the ticket complexity. Substantial: an autonomous Opus-class /ce:plan + /ce:work + /ce:review can be 10s of K turns and \$1–\$10 of subscription budget per ticket.
- **Trigger surface:** every comment on every open issue in the configured repos. Comments from anyone — operator, reviewer, bot, accidental webhook replay. Per-comment classification is the only gate.

## When this is correct

Most cases. The architect → comment → autodispatch loop is a feature, not a bug. The specific properties that make it *right*:

- The comment is a **complete spec** (Approach + Files + Verification + AC). Implementer can act without further clarification.
- The repo has a working `/mika` pipeline that catches plan-implementation drift downstream (/ce:plan would refuse a malformed plan; /ce:review catches bad code).
- The architect's GROOMED verdict has been recorded — implementing isn't speculation.
- The branch callout is canonical, so the worktree comes up correctly without re-derivation.

## When this can go wrong

- **Capture-only comments.** Posting an architect verdict, a status update, or a "noting this for later" comment fires the dispatch when no implementation was intended. Today's case is a benign instance: the dispatch was wanted, just not yet.
- **Iteration mid-flight.** If you post an ITERATE-class architect comment with revised plan content, mika-dev may dispatch on the revised content while the prior dispatch is still running. **Real risk** — leads to two parallel claude-pilot sessions on the same issue. The plan groomed today flagged this in the dispatch-guard ticket #807.
- **Replay attacks / webhook misconfiguration.** A replayed `issue_comment.created` could re-trigger a dispatch hours after the original. Less of a concern with mika's gateway (which dedups), but worth knowing.
- **External commenters.** A comment from an account other than known operators can still fire the autodispatch unless the classifier filters by author. Worth verifying — when the architect GROOMED comment from samidarko fires, it should be the *content* not the *author* that triggers; the same comment from `random_drive_by_contributor` should also fire (or get filtered).

## Mitigations available

In priority order, with their tradeoffs:

1. **Author-allowlist on the classifier.** Only treat comments from a known operator-set as actionable. Cheap to add; explicit signal. Loses some autonomy (PRs from external contributors won't trigger).
2. **Explicit invocation phrases.** Require comments to contain `@mika-dev implement` or similar phrase to fire dispatch. Loses the "spec-as-comment" elegance but eliminates the surprise.
3. **Dispatch-guard prompt** (already filed as `senara-solutions/mika#807`). Teach mika-dev's classifier to distinguish dispatch-intent from observation. The hard cases this needs to solve: review verdicts (GROOMED / READY) — are they spec or comment? Architect plans posted as comments — what about plans posted by humans? The classification problem is genuinely hard.
4. **Comment status-tag protocol.** Operator convention: lead actionable comments with a `> - **Status:** dispatch-now` callout (matching the canonical branch-callout grammar from the [grooming-branch compound](./grooming-branch-callout-required-2026-04-25.md)). The classifier looks for the explicit tag. Keeps spec-as-comment but requires a one-line marker. **Most aligned with the existing convention.**
5. **In-flight task detection.** Before firing a new dispatch, mika-dev checks the issue for existing in-progress tasks (`list_tasks(reference_url=…)`). If one exists, the new comment is treated as augmentation, not a new dispatch. Solves the iteration-mid-flight risk specifically.

Likely path forward: combination of (4) status-tag protocol + (5) in-flight task detection. The status-tag is operator-discipline; the in-flight check is system-discipline. Together they cover both "I didn't mean to fire" and "I fired twice."

## Concrete operator discipline (effective immediately, no code change)

Until the dispatch-guard work lands, operators should:

- **Treat every comment on an open issue as a potential dispatch.** Default assumption: if it has Approach/Files/Verification sections, it will fire claude-pilot. Don't post draft specs as comments.
- **Use the issue body for capture artifacts.** Edit the body, not a comment, for "noting this here for later" content. The body is read at dispatch time but body-edits do not fire `issue_comment.created`.
- **Use a draft section in the body, then promote to comment.** When the plan is GROOMED and ready to dispatch, post the comment. The comment posting *is* the dispatch.
- **Check before posting if a task is already running.** `gh issue view <n> --comments` and `mika tasks list` before posting an iterate-style comment — if claude-pilot is already running, hold the comment until the prior run lands.

## Loop closure observation

This was the **first observed end-to-end architect-implementation loop closing entirely autonomously**:

```
mika-arch grooms plan
   ↓ (GROOMED verdict in conversation memory)
operator copies to GitHub comment
   ↓ (issue_comment.created webhook)
mika-dev autodispatches
   ↓ (run_claude_pilot)
claude-pilot starts /mika in correct worktree
   ↓ (canonical branch callout from issue body)
… plan, work, review, PR
```

End-to-end from architect's GROOMED verdict to claude-pilot running: ~5 minutes including comment composition. The operator's role was: paste, post, observe.

This is what mika-arch v1 was building toward — a **fully autonomous architect-validated implementation pipeline** where human time is spent on architectural disagreement (escalations, rulebook updates), not on dispatch ceremony. Today the loop closed for the first time. The discipline questions above ("when is this correct" / "when does it go wrong") become the steering wheel for the next iteration.

## Update 2026-04-26 — QA-approval event is also a trigger

Confirmed second instance of the same broader pattern with a *different trigger event*. mika-dev session `9cc0a1d3-9dd1-49a0-8594-0cc10b3cd234` (channel: `github`) fired on a `pull_request_review.submitted (APPROVED)` webhook for PR #819 (mika#817's implementation). Within ~45 seconds mika-dev had:

1. Run `list_tasks(in_progress)` → `list_tasks(pending)` → `list_tasks()` — backlog scan.
2. Picked mika#818 (the next groomed-but-not-dispatched ticket, with a Branch + Plan callout + GROOMED architect verdict on its body).
3. `create_task` → `update_task_status(in_progress)` → `run_claude_pilot(prompt: "mika#818")`.

Crucially: this happened **40 seconds before PR #819 actually merged.** The QA-approval event was sufficient signal for mika-dev to "make progress on the next thing" — the merge itself wasn't required. The operator's intent ("wait for #817 to land before dispatching #818") existed only in conversation context with CC and as the text "but gated on mika#817 ... landing first" in #818's GROOMED summary comment. **Neither was parseable by the classifier as a hold signal.**

Generalization: the trigger event isn't just `issue_comment.created` (yesterday's compound). Any GitHub event mika-dev's classifier reads as "actionable progress" — comment, QA approval, CI success, possibly merge — can fire backlog-scan + autodispatch on the next groomed ticket. The operator-side gate must be encoded in something the classifier reads, not in conversation prose.

**Encoding options for explicit gating** (none implemented today; pick one when ready to ship the discipline):
- A label like `gated-on:mika#NNN` that the classifier checks before dispatching.
- A canonical body callout like `> - **Status:** gated-on-mika#NNN` parallel to the existing `> - **Branch:**` callout.
- A working-note in mika-dev's core memory recording open gates (similar to the operator-proxy memory-seeding pattern established for mika-arch on 2026-04-26).

Operationally for mika#818: no harm — #818 and #817 touch different files (`well_known_agents.rs` vs `builtin_handlers.rs`), no code dependency. The 40-second window didn't matter. But the pattern is now confirmed: this is how the autonomous loop closes by default; if you want to gate it, gate it explicitly.

## Related

- [`grooming-branch-callout-required-2026-04-25.md`](./grooming-branch-callout-required-2026-04-25.md) — the canonical issue-body callout pattern. Honored cleanly by the autonomous dispatch today; the worktree came up on the right branch without re-derivation.
- [`mika-arch-first-dogfood-2026-04-25.md`](../best-practices/mika-arch-first-dogfood-2026-04-25.md) — the architect-side of the loop closure. mika-arch session `f02d372e-…` produced the GROOMED verdict that fed this dispatch.
- `senara-solutions/mika#807` — self-dev dispatch-guard prompt (the classifier work that would distinguish "actionable spec" from "discussion comment").
- `senara-solutions/mika#814` — the ticket that was autonomously dispatched. claude-pilot subprocess `5f1e29db-…` is implementing it as of this writing.
- `senara-solutions/mika#803`, `#804` — long-running retry primitive + self-dev `error_max_turns` handler. Adjacent reliability gaps in the dispatch loop. Worth picking up alongside the dispatch-guard work.
- mika-dev session `e26d6bb9-922a-412e-89f7-f9eea4133ec8` — the canonical reference for inspecting the autonomous-dispatch tool sequence in DB.

## Resolution

**Ticket:** [mika#841](https://github.com/senara-solutions/mika/issues/841) — positive-consent dispatch gate via `ready` label.

**Origin:** mika#798 (2026-04-25) — first observed instance of a GitHub comment triggering unintended autonomous dispatch.

**Recurrence:** mika#838 (2026-04-27) — same failure shape. The `/mika-groom-ticket` Phase-5 closing comment contained a verbatim `implement mika issue#838` substring. The `issue_comment.created` webhook delivered this to mika-dev, whose Layer 1 routing table matched the embedded content, bypassing the Webhook Fallthrough scope rule.

**Initial proposed solutions (superseded):**
- mika#807 — three-source contract tightening via skill-prompt rules (negative-detection). Closed as superseded by mika#841.
- mika#801 — `check_active_grooming(issue, repo)` heuristic helper (negative-detection). Closed as superseded by mika#841.

**Final resolution — positive-consent dispatch gate (mika#841):**

Vincent's two-way rule: mika-dev dispatches if and only if (a) Vincent prompts mika-dev directly (via `mika ask`) OR (b) the `ready` label is set on the ticket. Everything else is inert — acknowledged at most, never dispatched.

Three layers changed:
1. **Gateway:** `route_event` admits `issues.labeled` events to mika-dev. `format_event_text` emits `[GitHub] Issue labeled <name> on <repo>#<n>` with the specific label name extracted from the webhook payload.
2. **Skill prompt (self-dev):** Layer 1 routing table gains a source-check precondition — matches only when the message has no bracketed source-prefix marker (`[GitHub]`, `[claude-pilot]`, `[Telegram]`). A new Ready-Label Dispatch handler fires on `[GitHub] Issue labeled ready on <repo>#<n>` — removes the label first (atomicity), then routes to Generic Workflow. Other label-add events fall through to Webhook Fallthrough (acknowledge only).
3. **Label taxonomy:** `ready` label added to `.github/labels.yml` with state-oriented description ("Approved for autonomous dispatch").

**Architectural argument (closure-bound vs completeness-bound):**

Negative-detection (mika#807/#801) was completeness-bound — heuristics fail on inputs they don't enumerate. A grooming signal not in the heuristic's deny-list bypasses the guard. Positive-consent is closure-bound — the rule enumerates the two valid dispatch triggers; everything else is inert by construction. Same allowlist-vs-denylist pattern from security: denylists fail the moment someone invents a new attack shape; allowlists hold by construction.

**Architect review:** mika-arch session `3801e5e4-5a7b-4d57-a9e0-f217964c913b` (lifecycle ESCALATE → operator-resolved as supersession); session `8959665a-7fa4-4e39-bc36-da48faf0d50d` (#841 first-pass ITERATE → revisions → second-pass GROOMED).
