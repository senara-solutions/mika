---
type: fix
ticket: mika#919
title: Engine-level grooming-marker guard at run_claude_pilot tool entry
status: drafted-pass2
related:
  - mika#907   # webhook-path prompt-level grooming check (kept as defense-in-depth)
  - mika#910   # webhook_no_unauthorized_dispatch — sibling engine-guard pattern
  - mika#864   # required_suffix_line — sibling engine-guard pattern
  - mika#525   # validate_dispatch_readiness — host function
  - mika#583   # per-class dispatch slot — composes with new check
  - mika#713   # blocked-by guard — sibling pre-spawn check pattern
  - mika#996   # auto-groom-on-dispatch — interacts with this guard at prompt level
  - mika#841   # ready-label canonical dispatch — webhook-path origin
  - feedback_prompt_enforcement_fragile.md
---

# fix: engine-level grooming-marker guard at run_claude_pilot tool entry

## Problem

Today the grooming-marker check that prevents un-groomed tickets from reaching
`claude-pilot` is **prompt-level**, wired into mika-dev's self-dev skill
(`skills/bundled/self-dev/system_prompt.md:253`) and triggered by the
`webhook_ready_label_dispatch` intent guard (`agent.rs:4574`). It fires only
when the inbound user message starts with the literal marker
`[GitHub] Issue labeled ready on ` (the webhook-path framing produced by
`mika-gateway` for `issues.labeled[name=ready]` events).

The CLI dispatch path — `mika ask --agent mika-dev "implement ..."` (single
ticket or sprint form) — does NOT carry that framing. The webhook intent guard
never fires; the self-dev prompt's grooming pre-flight is never invoked. The
operator can therefore ship code on a ticket that has zero architect review
and no plan committed on a branch.

The mika#908 dispatch on 2026-05-01 is the documented reproduction case: four
tickets shipped in a single CLI-sprint with no grooming pass on any of them.

The premise of mika#907 — "ungroomed dispatch should be structurally
impossible" — only holds for one of two real-world dispatch surfaces. Defense
in depth requires lifting the check from prompt-level to engine-level, at a
point all dispatch paths share.

## Phase 0 — Pinned source

The plan inserts a new check into `validate_dispatch_readiness()` and uses
helpers from three pinned locations. Each is quoted verbatim below for
implementer reference.

### `validate_dispatch_readiness` signature and ordering

`crates/mika-agent/src/skills/executor.rs:775` (function ends 956):

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
    tool_input: Option<&serde_json::Value>,
) -> Result<String, String> {
```

Current check ordering inside the function:

1. Task existence (lines 782–801).
2. Task status `pending|in_progress` (lines 803–818, returns `task_not_dispatchable` otherwise).
3. No active callback children (lines 820–855, returns `task_active_dispatch` otherwise).
4. Per-class dispatch slot free (lines 857–904, returns `global_dispatch_active` otherwise).
5. GitHub blocked-by check (lines 906–953, returns `dispatch_blocked_by` otherwise — gated on `task.reference_url` parsing as `GitHubRef::Issue`).

The `github_token` parameter is `Option<&str>`; the existing blocked-by check
**fail-open**s on `None` with a `warn!` log (lines 946–951), and **fail-close**s
on API error (lines 931–944, returns `dispatch_check_failed`).

### `task.r#type` field

`crates/mika-agent/src/db.rs:84-90`:

```rust
pub const TASK_TYPE_ISSUE: &str = "issue";
pub const TASK_TYPE_MILESTONE: &str = "milestone";
pub const TASK_TYPE_PROJECT: &str = "project";
pub const VALID_TASK_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];
```

`crates/mika-agent/src/db.rs:167-168`:

```rust
/// Task type discriminator (issue/milestone/project). Stored as TEXT,
/// validated by the DB CHECK constraint enforcing membership in
/// [`VALID_TASK_TYPES`].
pub r#type: String,
```

The new bypass predicate compares `task.r#type` against `TASK_TYPE_ISSUE`
(value `"issue"`).

### `GitHubRef::Issue` parse path

`crates/mika-agent/src/tools/check_task.rs:34-78` defines
`parse_github_ref(url: &str) -> Option<GitHubRef>`:

```rust
pub(crate) fn parse_github_ref(url: &str) -> Option<GitHubRef> {
    // ...
    match kind {
        "pull" => Some(GitHubRef::PullRequest { owner, repo, number }),
        "issues" => Some(GitHubRef::Issue { owner, repo, number }),
        _ => None,
    }
}
```

Returns `GitHubRef::Issue { owner: String, repo: String, number: u64 }` for
URLs matching `https://github.com/{owner}/{repo}/issues/{number}`. The new
check reuses this parse — it does NOT introduce a new URL parser.

Note: `parse_github_ref` is currently `pub(crate)`. The new gate runs in
`crates/mika-agent/src/skills/executor.rs`, the same crate, so no visibility
change is needed.

### `derive_dispatch_class` and `extract_skill_from_input`

`crates/mika-agent/src/skills/executor.rs:755-765`:

```rust
fn derive_dispatch_class(skill: Option<&str>) -> &'static str {
    match skill {
        Some("dev-groom") => "groom",
        _ => "implement", // dev-pilot, deploy_mika, and all others
    }
}

fn extract_skill_from_input(input: &serde_json::Value) -> Option<&str> {
    input.get("skill").and_then(|v| v.as_str())
}
```

The new bypass predicate uses `extract_skill_from_input(input)` to compare the
skill value against `"dev-pilot"` literally (NOT against `derive_dispatch_class
== "implement"`, because the latter would gate `deploy_mika` and any future
`implement`-class skill — out of scope for #919).

### Issue body fetch primitive

`crates/mika-agent/src/tools/check_task.rs:85-117` already implements
`github_get(token, url) -> Result<Value, String>`:

```rust
async fn github_get(token: &str, url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        // ...
}
```

It is currently `async fn` (private). The new `fetch_issue_body` helper
described below reuses this exact request shape — same auth, same headers,
same timeout, same error taxonomy — for consistency.

## Insertion point

The new grooming-marker check lives inside `validate_dispatch_readiness()`,
positioned **after** the per-class dispatch slot check (current step 4 in the
Phase 0 ordering above) and **before** the blocked-by check (current step 5).
Rationale: cheap DB checks first, expensive GitHub API calls last (same
ordering principle the function already uses). The marker check and the
blocked-by check both consume the same parsed `GitHubRef::Issue` and the same
`github_token`, so co-locating them simplifies the diff.

```
                        current order                       new order
                        -------------                       ---------
1. task exists                                          1. task exists
2. task status pending|in_progress                      2. task status pending|in_progress
3. no active callback children                          3. no active callback children
4. per-class dispatch slot free                         4. per-class dispatch slot free
5. blocked-by check                                     5. grooming-marker check      ← new
                                                        6. blocked-by check
```

The blocked-by check at lines 906–953 hoists `task.reference_url` parsing
into a local `if let Some(GitHubRef::Issue { owner, repo, number }) = ...`
block. The new grooming check needs the same parse. Implementation hoists
the parse above both checks so they share the binding:

```rust
let github_ref = task
    .reference_url
    .as_deref()
    .and_then(parse_github_ref);

// (5) Grooming-marker check — uses github_ref + tool_input + github_token
if let Some(GitHubRef::Issue { owner, repo, number }) = github_ref.as_ref() {
    grooming_marker_check(/* ... */)?;
}

// (6) Blocked-by check — same destructure, uses owner/repo/number
if let Some(GitHubRef::Issue { owner, repo, number }) = github_ref {
    blocked_by_check(/* ... */)?;
}
```

This is a small refactor (move the destructure up), not a structural change.

## Predicate (three-signal, body-only)

The mika#919 issue body specifies the grooming-marker check as a three-signal
shape: `> - **Branch:**` + `> - **Plan:**` + `Disposition: GROOMED` or
`Verdict: GROOMED` in a comment authored by mika-arch.

Implementing the literal three-signal shape with a comment-authorship check
adds a second GitHub API call (`gh issue view --comments`) and an
authentication-identity dependency on mika-arch's machine-user GitHub login.
mika-arch is read-only and does not write GitHub comments today; the
`/mika-groom-ticket` orchestrator (operator-driven) is the actual writer.
Comment-authorship is therefore the wrong anchor.

The `/mika-groom-ticket` canonical Phase 5 step 19 callout block writes
**all three signals into the issue body** (verified in
`.claude/commands/mika-groom-ticket.md:151-160`):

```
> - **Branch:** `<slug>`
> - **Plan:** `<repo>/docs/plans/<file>` (committed on branch @ `<sha>`)
> - **Grooming history:** /ce:plan → mika-arch first-pass (<disposition>) → revisions → mika-arch second-pass (GROOMED)
```

So the body-only three-signal check honors the issue's criterion using the
same API budget as a single-signal body check. All three predicates are
substring matches on the fetched issue body:

| # | Predicate                  | Match string                |
|---|----------------------------|-----------------------------|
| 1 | Branch callout             | `> - **Branch:**`           |
| 2 | Plan callout               | `Plan: docs/plans/`         |
| 3 | Architect-verdict callout  | `second-pass (GROOMED)`     |

All three must match. The `Plan: docs/plans/` substring is unchanged from the
existing prompt-level check (parity preserved). The `second-pass (GROOMED)`
substring is the unique architect-verdict signature emitted only by
`/mika-groom-ticket` Phase 5 — body-anchored, no auth-identity dependency,
robust against manual-edit attacks because forging it requires reproducing
the full canonical callout shape.

**Documented deviation:** the issue body says "or `Verdict: GROOMED` in a
comment authored by mika-arch." This plan uses the body-only `second-pass
(GROOMED)` substring instead. The reasons (no comment-author identity
coupling, same API budget) are above. The plan deviates from the literal
text but honors the intent (three-signal architect-verdict gate).

If `/mika-groom-ticket`'s canonical Phase 5 callout shape changes, BOTH the
engine guard and the prompt-level check (`skills/bundled/self-dev/
system_prompt.md:253`) must update together. The match strings are
substring-anchored, so minor reformatting (whitespace, bullet syntax)
survives — but the directory prefix `docs/plans/` and the literal phrase
`second-pass (GROOMED)` are load-bearing. Add a citation comment in both
files linking them as a coupled pair (R3 in Risks below).

## Bypass predicates

The check short-circuits with `Ok(...)` (no rejection) under any of:

1. `extract_skill_from_input(tool_input) != Some("dev-pilot")` — gates only
   `dev-pilot` dispatches. `dev-groom` is the producer of the marker;
   `deploy_mika` and any other future `implement`-class skill are out of
   scope for #919 (separate tickets if extension is wanted).
2. `task.r#type != TASK_TYPE_ISSUE` — milestones and projects do not carry
   plans on their own bodies. Sub-issue children are dispatched with
   `type: "issue"` per self-dev `system_prompt.md:597` and `:776` (verified)
   and trigger the gate naturally when they reach `run_claude_pilot`.
3. `task.reference_url` does not parse as `GitHubRef::Issue` (i.e.
   `parse_github_ref(url)` returns `None` or `Some(GitHubRef::PullRequest)`)
   — free-text or non-GitHub-issue tasks cannot have an issue body to check.
   See F4 in Risks below for the documented gap and tracking.
4. `std::env::var("MIKA_DISPATCH_BYPASS_GROOMING_CHECK")` is `"1"` or `"true"`
   (case-insensitive). Emergency operator unblock. Emits
   `warn!(target = "mika::executor", agent_id, task_id, repo, number,
   "dispatch grooming marker check bypassed via env var")` on every hit.

## API call and helper

Add a public helper at `crates/mika-agent/src/github_graphql.rs` (the module
where mika#713/#714 helpers already live):

```rust
/// Fetch the body of a GitHub issue via the REST API.
///
/// Returns the raw body text on success. Used by the dispatch-readiness
/// grooming-marker check (mika#919).
///
/// Reuses the same auth/header/timeout shape as `tools::check_task::
/// github_get` for consistency with the existing GitHub HTTP layer.
pub async fn fetch_issue_body(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<String, GraphqlError>;
```

Implementation calls `GET https://api.github.com/repos/{owner}/{repo}/issues/
{number}`, parses the response as `{ body: String }`, returns the body text.
Same error taxonomy as the rest of the module (`Network`, `HttpStatus`,
`Parse`).

REST not GraphQL: the body is a single scalar field, REST returns just
`{body: "..."}` with no query authoring cost. The module name
`github_graphql` is historical — it already hosts REST helpers internally.

## Rejection shape

When any of the three predicates fails, return the canonical structured
error:

```rust
Err(serde_json::json!({
    "error": "dispatch_no_grooming_marker",
    "task_id": task_id,
    "issue": format!("{}/{}#{}", owner, repo, number),
    "missing_signals": missing_signals_vec, // e.g. ["branch_callout", "groomed_verdict"]
    "predicate": "issue body must contain all three substrings: \
                  '> - **Branch:**', 'Plan: docs/plans/', 'second-pass (GROOMED)'",
    "recovery": "Run /mika-groom-ticket <ref> to produce the canonical \
                 callout block, or dispatch dev-groom first via \
                 'mika ask --agent mika-dev \"groom <typed-ref>\"', or \
                 set MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1 to bypass.",
    "reason": format!(
        "Cannot dispatch dev-pilot on ticket #{number}: issue body is \
         missing one or more grooming-marker signals. The grooming-marker \
         gate ensures architect-reviewed plans are committed before \
         implementation begins (mika#907, mika#919)."
    )
})
.to_string())
```

The error code `dispatch_no_grooming_marker` is the named contract the
issue's acceptance criteria call for. The LLM sees this in the tool result
and routes to either grooming or operator notification per the self-dev
prompt's existing handling for `validate_dispatch_readiness` errors.

**`missing_signals` field** lists which of the three checks failed. Useful
for debugging and for the operator notification path — the recovery hint
can be tailored ("run /mika-groom-ticket" vs "your Phase 5 callout dropped
the Branch line"). Cheap to compute (already iterated during the check),
no extra cost.

mika#1011's deferred-dispatch pattern is NOT applicable here: there's no
retry semantic; the gate is terminal until the ticket is groomed.

## Failure modes — fail-closed vs fail-open

The existing blocked-by check (mika#713 / lines 906–953) uses a split policy:

- **No `github_token` configured** → fail-open with `warn!` log, skip check.
- **`github_token` present but API error** → fail-closed, return
  `dispatch_check_failed`.

The grooming-marker check **mirrors this exactly**. Rationale:

- **Missing token (fail-open):** agent operator did not configure a GitHub
  token. The agent cannot fetch the body at all; rejecting every dispatch
  would brick the agent. The prompt-level check (mika#907) still fires on
  the webhook path, so defense-in-depth is preserved.
- **API error with token present (fail-closed):** transient GitHub 5xx or
  rate-limit could theoretically kill an in-progress sprint queue. But
  fail-open would silently re-open the bypass that the entire gate exists
  to close. The asymmetry is correct: the gate's purpose ("structurally
  impossible to ship un-reviewed code") trumps sprint throughput, and the
  operator can re-enqueue manually after the API recovers. The bypass env
  var is the documented escape hatch for sustained API outages.

This matches mika#713 precedent exactly. The documented cost asymmetry the
brief flagged (R5 / F5 in the architect review) is resolved by adopting
mika#713's split policy verbatim — same shape, same trade-off.

## Bypass env var

`MIKA_DISPATCH_BYPASS_GROOMING_CHECK` follows the existing `MIKA_DISABLE_*`
naming convention (`MIKA_DISABLE_BUNDLED_SKILLS`,
`MIKA_DISABLE_AGENT_PROVISIONING` at `agent.rs:1670`, `:1698`). Truthy
values: `"1"`, `"true"` (case-insensitive). Any other value (including
unset) means the gate fires.

Logged at `WARN` with structured fields (`agent_id`, `task_id`, `owner`,
`repo`, `number`) every time it's hit — intentionally noisy. Bypass usage
should be visible in retrospective review.

Documented in:

- `crates/mika-agent/CLAUDE.md` — extend the "Dispatch-readiness guard
  (#525)" paragraph and the env-var list.
- `docs/runtime-structure.md` — canonical env-var reference table.

## Code locations

| Path | Change |
|---|---|
| `crates/mika-agent/src/skills/executor.rs:775-956` | Add grooming-marker check inside `validate_dispatch_readiness`. Hoist `parse_github_ref(task.reference_url)` above both the new check and the blocked-by check (small refactor — share the destructure). |
| `crates/mika-agent/src/github_graphql.rs` | Add `pub async fn fetch_issue_body(token, owner, repo, number) -> Result<String, GraphqlError>`. |
| `crates/mika-agent/CLAUDE.md` | Update the "Dispatch-readiness guard (#525)" paragraph to enumerate the marker check as the 5th gate (renumber blocked-by to 6th); add `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` to the env-var list. |
| `docs/runtime-structure.md` | Add bypass env var to the canonical env-var reference. |
| `skills/bundled/self-dev/system_prompt.md:253` | Add a citation comment linking the prompt-level marker check to the engine-level guard at `crates/mika-agent/src/skills/executor.rs` (coupled pair — both must update if the canonical callout shape changes). No behavioral change. |
| `crates/mika-agent/tests/eval/test_dispatch_no_grooming_marker_guard.rs` | New behavioral test file mirroring `test_ready_label_grooming_guard.rs`. |

## Tests

Behavioral tests in `tests/eval/test_dispatch_no_grooming_marker_guard.rs`,
mirroring the harness pattern used by `test_ready_label_grooming_guard.rs`
and `test_webhook_no_unauthorized_dispatch_guard.rs`. All use `EvalHarness`
+ `MockLlmProvider` + mock GitHub HTTP client.

Scenarios (each a separate `#[tokio::test]` function):

1. **`ungroomed_dev_pilot_rejects`** — Task with `reference_url` →
   `senara-solutions/mika/issues/919`, body lacks all three signals; LLM
   calls `run_claude_pilot{skill="dev-pilot", task_id: <uuid>}`. Assert:
   tool result JSON contains `"error":"dispatch_no_grooming_marker"` and
   `missing_signals` lists all three.
2. **`partial_marker_dev_pilot_rejects`** — Body has `Plan: docs/plans/foo`
   but no `> - **Branch:**` or `second-pass (GROOMED)`. Assert: rejects
   with `missing_signals: ["branch_callout", "groomed_verdict"]`.
3. **`fully_groomed_dev_pilot_proceeds`** — Body contains all three
   signals. Assert: gate returns `Ok(...)`; rejection if any occurs comes
   from downstream (not the marker gate).
4. **`ungroomed_dev_groom_proceeds`** — Same ungroomed body, LLM calls
   `run_claude_pilot{skill="dev-groom", ...}`. Assert: gate bypassed via
   skill predicate.
5. **`milestone_type_bypasses`** — `task.r#type == "milestone"`. Assert:
   gate bypassed via task-type predicate.
6. **`non_issue_reference_bypasses`** — `task.reference_url` points to a
   PR (`/pull/123`), not an issue. Assert: gate bypassed via parse
   predicate.
7. **`no_github_token_warns_and_bypasses`** — `github_token: None`. Assert:
   gate bypassed, WARN log emitted (mirror blocked-by behavior).
8. **`gh_api_error_fail_closed`** — Mock `fetch_issue_body` returns
   `Err(GraphqlError::HttpStatus(503))`. Assert: gate returns
   `Err({"error":"dispatch_check_failed", ...})` (NOT
   `dispatch_no_grooming_marker` — surface the cause to the LLM).
9. **`env_bypass_skips_check`** — Ungroomed body + dev-pilot skill +
   `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1`. Assert: gate bypassed, WARN
   log emitted. Use `serial_test::serial` to avoid env-var contamination.

The acceptance criterion "rejection covers all four trigger paths
[ready-label webhook, single CLI ask, sprint CLI ask, free-text]" is
satisfied **structurally**: all four paths converge on
`execute_skill_tool` → `validate_dispatch_readiness`, so unit-testing the
guard at that entry point covers all four. Scenario #1 above is the
canonical witness; explicit per-path scenarios would be redundant and
brittle.

## Sequencing

p1-important per ticket label. Single PR, mika repo only. No mika-cloud,
mika-skills, or claude-pilot changes. Behavioral tests guard the gate;
existing `test_ready_label_grooming_guard.rs` continues to cover the
prompt-level defense-in-depth path (unchanged).

Estimated diff: ~180 LOC core (executor.rs + github_graphql.rs) + ~350
LOC tests + ~40 LOC docs.

## Acceptance criteria mapping

The ticket lists six AC. Mapping to this plan:

| AC | This plan |
|---|---|
| `run_claude_pilot` rejects with named error when issue body lacks grooming callouts | Insertion in `validate_dispatch_readiness`; error code `dispatch_no_grooming_marker`. |
| Rejection covers all trigger paths (ready-label webhook, mika ask, sprint, free-text) | Structurally — all paths funnel through `validate_dispatch_readiness`. See F4 for the documented free-text gap (no-`reference_url` skip-not-reject; tracked separately). |
| Existing webhook-path check (mika#907) continues to fire | Unchanged. Prompt-level check at `system_prompt.md:253` retained as defense-in-depth. |
| Behavioral test: ungroomed dispatch via each path rejects | Scenarios 1–2 (structural witness — covers all paths per the convergence above). |
| Behavioral test: groomed dispatch via each path succeeds | Scenario 3 (structural witness). |
| Operator override path (env var, WARN log) | `MIKA_DISPATCH_BYPASS_GROOMING_CHECK`; scenario 9. |

## Out of scope

- Refactoring or removing the prompt-level grooming check at
  `skills/bundled/self-dev/system_prompt.md:253`. Kept as defense-in-depth.
- Auto-promoting `dev-pilot` to `dev-groom` on missing marker (the mika#996
  auto-groom-on-dispatch flow). The engine gate REJECTS; the LLM caller
  decides whether to recover via `dev-groom` or notify the operator.
- Comment-authorship check (`Verdict: GROOMED` in a mika-arch-authored
  comment). The body-only three-signal predicate honors the issue's intent
  with no auth-identity coupling.
- Extending the gate beyond `dev-pilot` (e.g. `deploy_mika`,
  `qa-review-build-callback`). Separate ticket if extension is wanted.
- Ticketless-dispatch enforcement (rejecting tasks with no `reference_url`).
  Tracked separately by `feedback_no_ticketless_dispatch.md` and F4 below.
- An explicit per-call CLI flag on `mika ask` for bypass. The env var
  covers operator unblock adequately.

## Risks and open questions

**R1 — Performance.** Adds one `gh api` HTTPS round-trip to every
`run_claude_pilot{skill="dev-pilot"}` invocation. Same latency order as the
existing blocked-by check; co-locating them in the fail-closed-on-error
block keeps rollback identical. Not called in tight loops.

**R2 — Token availability.** Resolved by the split fail-open/fail-closed
policy above (no token → fail-open with WARN, mirroring mika#713; token
present but API error → fail-closed).

**R3 — Schema drift on the marker substrings.** If `/mika-groom-ticket`
ever changes the canonical Phase 5 callout shape, BOTH the engine guard
and the prompt-level check must update together. Mitigation: the predicates
are substring matches (not full-shape regexes), so minor reformatting
survives. The directory prefix `docs/plans/`, the literal `> - **Branch:**`
callout, and the phrase `second-pass (GROOMED)` are the load-bearing
invariants. A citation comment is added to `system_prompt.md:253` and the
new guard linking them as a coupled pair.

**R4 — Cache or no cache?** Two `run_claude_pilot` calls on the same issue
within a short window would each fetch the body. Rejected: TTL/invalidation
state-management exceeds the cost of one extra API call. No cache for MVP.

**R5 — Bypass abuse.** `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1` set in the
service environment would silently disable the gate. Mitigation: WARN log
on every hit, surfaced in retrospective review. A future ticket can add a
server-startup INFO line when bypass is set at boot, but that's polish.

**R6 — Interaction with mika#996 auto-groom-on-dispatch (architect F3
follow-up).** mika#996 runs in the self-dev prompt: when the ready-label
handler detects a missing marker (Step 3 of `system_prompt.md:245-272`), it
dispatches `dev-groom` first, waits for callback, then re-enters and
dispatches `dev-pilot`. The dev-groom session writes the three callouts to
the issue body via `gh issue edit` in Phase 5 step 18 of
`/mika-groom-ticket`. The engine gate then fires on the second
`run_claude_pilot{skill="dev-pilot"}` call.

  **Re-entry timing.** Between dev-groom's `gh issue edit` (REST PATCH,
durable on 200 OK) and mika-dev's re-entry `gh issue view` (REST GET on
the same resource), several seconds elapse (claude-pilot session
teardown + callback persistence + Silent::Callback dispatch + agent
turn). GitHub's eventual-consistency window for a single-resource
PATCH→GET on the same primary store is sub-second in practice.

  **Mitigation.** No retry/backoff loop. Fail-closed is the correct
behavior: if the rare race fires, the operator re-adds the `ready` label
and the entire flow restarts via the existing re-entry mechanism. Adding
retry-on-marker-miss-after-dev-groom-completion increases complexity for
a sub-second race window — not worth it.

**F4 — Free-text dispatch gap (architect non-blocking).** Today the gate
skips when `task.reference_url` doesn't parse as `GitHubRef::Issue`. This
leaves free-text dispatches (tasks created without a `reference_url`)
ungated. `feedback_no_ticketless_dispatch.md` says "Never dispatch
free-text to mika-dev without a GitHub issue; file the ticket first."
Strict reading of the AC suggests this should reject. The plan adopts
**skip-not-reject** for free-text and flags the gap explicitly:

  Track at the platform level via the existing
`feedback_no_ticketless_dispatch.md` policy. If a structural enforcement
is wanted, file a follow-up ticket `mika: reject run_claude_pilot when
task has no GitHub reference_url` that extends this gate with a fourth
predicate. Not in scope for #919.

**F6 — Milestone-cascade trace (architect non-blocking, now verified).**
Verified: self-dev `system_prompt.md:597` and `:776` show milestone- and
project-cascade `create_task` calls explicitly pass `"type": "issue"` on
the sub-issue children. The parent milestone/project task with
`type: "milestone"` or `type: "project"` is never directly dispatched to
`run_claude_pilot` — the cascade decomposes first. The bypass predicate
`task.r#type != TASK_TYPE_ISSUE` is therefore safe and does not create a
milestone-shaped bypass for sub-issue dispatches.

**Open Q1 — Token check first (fail-open) or marker fetch unconditionally
(fail-closed on missing token)?** The plan picks split policy mirroring
mika#713 (no token → fail-open, API error → fail-closed). This is
load-bearing for production agents without a GitHub token (e.g. CLI test
agents). Confirmed via architect F5 ratification path.
