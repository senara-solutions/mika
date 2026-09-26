---
issue: 716
type: fix
title: "callback-error verified-state pre-injection (grounded-by-construction)"
branch: fix/716/mika-dev-fabricates-state-claims-on
authored_against: mika @ 1688ab26
companion: "#1331 (general assert_grounded EndTurn guard — WC1 universal backstop)"
---

# Plan: callback-error verified-state pre-injection (#716)

## What changed since the 2026-05-27 plan (read this first)

This plan **supersedes** `2026-05-27-001-fix-716-callback-error-state-verification-plan.md`
(committed at `ebb96e8c`, predates current substrate). That plan proposed a **two-layer**
fix whose Layer 1 was a `detect_unverified_callback_state_claim()` **regex brake** keyed on
fabrication vocabulary ("no PR", "manually closed", "handler crashed").

That Layer-1 is retired here for two reasons grounded in this week's substrate changes:

1. **It is the exact shape just removed.** mika#1322's fabrication-string brake was retired
   on 2026-05-28 (`1688ab26` / #1328) precisely because text-match brakes false-positive on
   honest pilots (3/3 on 2026-05-28). A new vocabulary regex would re-introduce the retired
   failure mode under a new name. State-checking, not text-matching, is the substrate's
   chosen discipline (`feedback_prompt_enforcement_fragile`).
2. **The general structural guard is now its own ticket.** The "affirmative ungrounded
   state-claim" guard — the *correct* structural backstop — is split out as **#1331**
   (`assert_grounded` EndTurn `IntentPrecondition`, state-checking not text-matching). #716
   no longer carries a guard at all.

**#716's refreshed scope is one thing:** pre-inject **verified GitHub state** into the
callback turn's context on the failure path, so "no PR" / "manually closed" claims are
**grounded by construction** — the LLM sees the real PR/issue state before it generates.
This is exactly what the issue body's *primary* "Proposed fix" already asks for ("Inject the
verified state into the callback framing so the LLM responds based on facts, not inference").

Relationship to #1331 (framing OQ4, resolved): **both are warranted, neither redundant.**
Pre-injection (#716) grounds the common callback-failure case *before* generation on the one
path where we can cheaply fetch authoritative state. The general guard (#1331) is the
universal *after-generation* backstop for webhook/conversation turns where pre-injection is
not feasible. Siblings, not a blocking dependency — they can ship in either order.

## Problem (unchanged from issue)

When a claude-pilot callback handler crashes or returns a failure signal, mika-dev's LLM
fabricates a narrative about downstream state (PR status, issue close reason) without calling
any tool to verify. Observed 2026-04-21 (sprint #713→#714): PR #715 was created, merged, and
auto-closed #713 — yet mika-dev reported *"handler crashed. Issue was manually closed without
PR."* Every clause was false. The agent had `run_gh` available and did not use it.

## Root cause (precise, re-verified @ 1688ab26)

The callback failure path delivers an **untrusted error result** to the LLM with framing that
*instructs* ("Report only what this result explicitly states. Do not infer the state of any
system…" — `format_callback_framing`, `agent.rs:170-177`) but provides **no verified ground
truth** to anchor against. Under an error signal the LLM rationalizes a plausible-but-false
story. Prompt instruction without ground truth is the `feedback_prompt_enforcement_fragile`
failure mode.

The existing EndTurn guards do not catch this shape:
- `detect_completion_claim` (#483, `agent.rs:5014`) — fires on *completion* claims; here the
  LLM claims *failure*.
- `detect_fabricated_action_claim` (#308, `agent.rs:5180`) — requires *zero* tool calls; the
  callback turn calls `update_task_status` + `send_message`, so `tools_called` is non-empty.
- `asserted_unavailability` (#862, `agent.rs:5848`) — catches *negative tool-availability*
  claims, not affirmative state claims.

The affirmative-state-claim shape falls through every guard. #1331 closes that gap generally;
#716 prevents the callback instance by construction.

## Fix: pre-inject verified GitHub state on the callback failure path

### Where (the load-bearing architectural fact)

`format_callback_framing` (`agent.rs:135`) and `build_callback_trigger_context`
(`agent.rs:99`) are **pure, synchronous string builders with 6+ callers** including the CLI
(`mika-cli/src/commands/chat.rs:378`) and five unit tests. They **must not** become async or
grow a GitHub-fetch dependency — that would force a token into every caller and break the CLI
path. **The pre-injection happens in the async callback turn assembly, not in the pure
formatter.**

The injection site is `run_silent_inner`'s callback branch:
- The `SilentTrigger::Callback { task_id, label, result, failed, parent_task_id }` match arm
  builds `trigger_context` via `build_callback_trigger_context` (`agent.rs:3486-3505`).
- The GitHub token is resolved in the same function at `agent.rs:3753`
  (`resolved_github_token`, via `settings.resolve_github_token(github_app)` — App-installation
  token preferred, PAT fallback). `SilentAgentParams` already carries `github_token` +
  `github_app` (`agent.rs:3371-3373`); the dispatcher already passes `self.github_token`
  (`dispatcher.rs:267,458`).

So the verified-state block is **appended to `trigger_context`** after token resolution. No
new params threaded; the token is already in scope.

### Step 1 — `fetch_verified_callback_state` helper (new, async)

New `async fn` (co-located with the other GitHub helpers — `github_graphql.rs`, or a small
private fn in `agent.rs` near the callback branch). Signature shape:

```rust
/// Fetches authoritative GitHub state for the resource a failed callback references,
/// so the callback turn is grounded by construction. Returns a pre-formatted
/// `<verified_github_state trust="data">…</verified_github_state>` block, or None when
/// state cannot be established (no token, unresolvable resource, API error) — in which
/// case the turn proceeds with existing untrusted-only framing (no regression).
async fn fetch_verified_callback_state(
    token: &str,
    repo: &str,          // "senara-solutions/mika"
    issue_number: u64,
) -> Option<String>
```

Implementation reuses existing, proven helpers — **no new GitHub plumbing**:
- Issue state: `github_graphql::fetch_issue_body` / a `gh issue view <n> --json state,stateReason,closedAt`
  call (mirror the existing `run_gh` issue-view shape).
- Associated PR: reuse **`find_open_pr(repo, branch, token)`** (`server/ci_success_handler.rs:385`,
  `pub(crate)`, returns `Option<PrInfo>` with `{ number, url, state, mergedAt, reviewDecision }`)
  where the branch is derivable from the callback context; **and/or** a
  `gh pr list --search <n> --state all --json number,state,mergedAt,url` for the merged-by-PR
  case (the #715 incident: PR was *merged*, not open, so an open-only query must not be the
  only path — search across states).

The returned block states facts only, e.g.:

```
<verified_github_state trust="data">
Issue senara-solutions/mika#713: state=CLOSED, stateReason=COMPLETED, closedAt=2026-04-21T14:33:09Z
PR #715: state=MERGED, mergedAt=2026-04-21T14:33:08Z, url=…
</verified_github_state>
This block is engine-verified ground truth. Describe only what it states.
```

**Trust-tier framing is deliberate** (resolves first-pass F4): the verified block uses
`trust="data"` and sits alongside the existing `<callback_result trust="untrusted">` block.
The `data` vs `untrusted` contrast is exactly the signal the LLM needs — `data` = engine-
verified factual ground truth, `untrusted` = claude-pilot output that may be wrong. No
alternative framing is needed; the contrast is the mechanism.

### Step 2 — resolve the referenced resource (repo + issue number)

The callback's `parent_task_id` → parent self_dev task → `Task.reference_url`
(`db.rs:86`, `Option<String>`, e.g. `https://github.com/senara-solutions/mika/issues/713`).
Parse with the existing URL-parse helpers (`github_graphql::parse_pr_url` and siblings; add a
trivial `parse_issue_url` if none matches the issues path) into `(repo, issue_number)`.

Resolution chain: `parent_task_id` present → `db.get_task(parent_task_id)` → `reference_url`
→ parse. If `parent_task_id` is `None`, or `reference_url` is `None`/unparseable, **skip
pre-injection** (return `None`, Step 3 no-ops). This is the dominant fail-open path and must
not error the turn.

**Why fail-open (not fall back to the callback task's own `reference_url`) when
`parent_task_id` is None** (resolves first-pass F2): the callback task's own `reference_url`
typically points to the **PR** (the pilot's artifact), not the originating **issue**. Looking
it up would yield PR state but miss issue state — and v1 grounding wants *both* (issue
state=CLOSED + PR state=MERGED is what disproves "manually closed without PR"). Partial
grounding off the wrong resource is worse than clean fail-open, which #1331 backstops
generally. So: no parent issue resolvable → no block.

### Step 3 — wire into the callback branch (failure path only, v1)

In `run_silent_inner`, after `trigger_context` is built and the token is resolved:

```rust
if let SilentTrigger::Callback { failed: true, parent_task_id: Some(pid), .. } = &params.trigger
    && let Some(token) = resolved_github_token.as_deref()
    && let Some((repo, n)) = resolve_callback_resource(db, pid).await
    && let Some(block) = fetch_verified_callback_state(token, &repo, n).await
{
    trigger_context.push_str("\n\n");
    trigger_context.push_str(&block);
}
```

Scope decisions:
- **`failed == true` only in v1 — design decision, not a deferral** (resolves first-pass F3).
  Success-path callbacks already carry verified state: the callback result contains the PR URL
  and the success path discovers PRs via `find_open_pr`. The **failure** path is the gap — the
  error result provides no verified state to anchor against. Failure-only scoping targets the
  exact incident class and bounds added GitHub calls; it is the correct v1 boundary, not a
  punt.
- **Fail-open everywhere.** No token, no parent, unparseable URL, API error, timeout → the
  turn proceeds with today's framing. Pre-injection only ever *adds* ground truth; it never
  blocks or errors a callback turn. This is the safety contract.
- **Bounded cost.** At most 2 `gh` calls (issue view + PR search) on the failure path only,
  inside the existing callback step/deadline budget. The watchdog/deadline machinery is
  unchanged.

### Step 4 — skill-prompt reinforcement (defense-in-depth, optional-but-cheap)

`skills/bundled/self-dev-callback/system_prompt.md` already carries prompt-level failure-path
rules (the issue notes `:99,126`). Add one line on the failure handler pointing the agent at
the injected `<verified_github_state>` block ("When an engine-verified state block is present,
it is ground truth — never contradict it"). This is prompt-level and *secondary* to the
structural pre-injection; it is not a brake and adds no regex.

## Behavior-test contract (pins the fix)

New scenarios in `crates/mika-agent/tests/eval/grounding_regressions/` (the suite already has
35 scenarios; follow `README.md` + the `INTENT_GUARDS` test discipline). The harness is
`EvalHarness` + `MockLlmProvider`; GitHub state is injected via the pre-formatted block, so
these tests do **not** need live network — they assert on the grounded prompt + response.

1. **Grounded-by-construction (the #716 incident replay).** Failed callback for #713; verified
   block states `PR #715 MERGED` + issue `CLOSED/COMPLETED`. Assert the response does **not**
   contain "no PR" / "manually closed" / "handler crashed" (forbidden-word assertion,
   `assert_response_forbids`) and that it reflects the merged PR.
2. **Fail-open, no regression.** Failed callback with no resolvable `reference_url` (or no
   token) → no `<verified_github_state>` block injected, turn completes exactly as today
   (assert block absent; assert turn still terminates with the required callback actions).
3. **Genuine no-PR failure.** Failed callback where the verified block legitimately shows
   *no* PR and issue still OPEN → "no PR" is now **grounded** and allowed (assert the response
   may state no-PR AND that it was preceded by the verified block — no false brake, mirroring
   the #1322-retirement lesson).

(The existing eval file `crates/mika-agent/tests/eval/grounding_regressions/callback_state_claim_unverified.rs`
staged in the worktree was written for the **retired** Layer-1 regex approach; it is superseded
by scenarios 1–3 above and should be removed/rewritten during implementation, not carried.)

## Files touched (estimate)

| File | Change | Lines (est.) |
|------|--------|--------------|
| `crates/mika-agent/src/agent.rs` (or `github_graphql.rs`) | `fetch_verified_callback_state` + `resolve_callback_resource` helpers | ~60 |
| `crates/mika-agent/src/agent.rs` | wire pre-injection into `run_silent_inner` callback branch | ~20 |
| `crates/mika-agent/src/github_graphql.rs` | `parse_issue_url` if no existing parser fits | ~15 |
| `skills/bundled/self-dev-callback/system_prompt.md` | one ground-truth reinforcement line | ~3 |
| `crates/mika-agent/tests/eval/grounding_regressions/` | 3 scenarios (replace stale `callback_state_claim_unverified.rs`) | ~120 |
| **Total** | | **~220** |

## Explicitly out of scope (→ #1331)

- The general `assert_grounded` EndTurn `IntentPrecondition` (affirmative-state-claim guard).
- Any text-vocabulary regex brake (retired shape; do not reintroduce).
- Webhook/conversation-turn grounding (the universal backstop is #1331).

## Test plan

1. `cargo test -p mika-agent --test eval -- grounding` — the 3 new scenarios.
2. `cargo test -p mika-agent` — full unit suite (the pure `format_callback_framing` tests must
   still pass unchanged — proof the formatter stayed pure/sync).
3. `cargo clippy` — lint.
4. Manual: trigger a `failed:true` callback against a real merged PR; confirm the
   `<verified_github_state>` block appears in the turn context and the agent reports the merge.

## Risks

- **Resource-resolution miss.** If `reference_url` is absent on some callback parents, those
  turns get no grounding. Acceptable: fail-open, no regression; #1331 backstops generally.
- **PR discovery for merged-by-PR.** The #715 case proves the query must search **all** PR
  states (the PR was merged, not open) — `find_open_pr` alone is insufficient; pair it with a
  state-agnostic `gh pr list --search`. Flagged for the implementer.
- **Known limitation — `gh pr list --search <n>` false-negative class** (resolves first-pass
  F1). `gh pr list --search <n> --state all` matches the issue number in PR **title/body**.
  It reliably catches the common case (the #715 incident PR title was the sprint ticket
  `#713 → #714`, so `--search 713` matches). It will **miss** PRs that close an issue purely
  via a `Fixes #N` / `Closes #N` body-trailer without mentioning the number in title/body
  text. The authoritative alternative is the GraphQL `closedByPullRequestsReferences` /
  `closingIssuesReferences` field. **#805 constraint check:** `run_gh`'s `gh api` is
  restricted to GET on two branch/commit endpoint patterns (`validate_gh_api_scope`), so the
  GraphQL POST path is **blocked** for the agent's `run_gh` today. The engine-side helper here
  is **not** subject to that skill-scoped restriction (it calls `run_gh_subprocess` directly,
  same as `find_open_pr`), so GraphQL is technically available to the helper if needed.
  **v1 decision:** accept the `gh pr list --search` heuristic — it covers the dispatch-loop's
  own PRs (which always title-reference the issue) — and leave a code comment pointing to the
  GraphQL `closingIssuesReferences` upgrade if the false-negative class is observed in
  practice. Fail-open means a missed PR degrades to "no verified PR block," never a false
  claim.
- **Added GitHub calls.** Bounded to ≤2 on the failure path; within callback budget.

## Related

- **#1331** — general `assert_grounded` guard (companion; universal backstop; framing OQ4).
- #1322 (retired, #1328/`1688ab26`) — the fabrication-string brake whose failure mode the
  retired Layer-1 would have recreated; the reason this plan is state-checking not text-matching.
- #466 (closed) — required tool verification before PR/merge claims (EndTurn path).
- #308 `fabricated_action_claim`, #483 `completion_claim` — adjacent EndTurn guards (don't fire here).
- Framing: `mika-platform/docs/brainstorms/2026-05-29-wc1-mika-dev-fabrication-structural-fix.md`.
- Memory: `feedback_prompt_enforcement_fragile`, `feedback_mika_dev_llm_fabricates_tool_errors`.
