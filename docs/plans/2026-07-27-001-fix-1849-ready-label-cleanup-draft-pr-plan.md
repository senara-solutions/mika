# Plan — fix(auto_pull): clean up `ready` label when a draft PR opens for its closing-issue

**Ticket:** mika#1849
**Type:** fix (loop-state hygiene, p1-important)
**Branch:** `fix/1849/auto-pull-clean-up-ready-label-when`
**Target file:** `crates/mika-agent/src/server/draft_pr_opened_handler.rs` (new) + wiring in `crates/mika-agent/src/server/handlers.rs`

---

## Context

When mika-dev/dev-pilot opens a **draft** PR closing an issue (systematically via
dispatch-lib's wip-rescue path, mika#1282), the issue's `ready` label is never removed. It
stays on the issue permanently, so loop inventory reads "N ready" when only a fraction are
actually dispatchable — the operator can no longer tell "needs dispatch" from "draft is open,
waiting on review". Frustrated Vincent 2026-07-27 morning ("11 drafts stackés + 5 ready
ambiguës").

Two contributing gaps (from the ticket root-cause):

1. **No handler listens for `pull_request.opened` (draft) and removes `ready`** from the issues
   that PR closes.
2. The Phase 2 auto-pull reconciler (mika#1824) *correctly skips* re-driving an issue that has an
   open PR closing it (Filter 2, `auto_pull.rs:664`), but it does **not** proactively *remove* the
   leftover `ready` label either.

**Chosen approach — Option A (webhook-driven), per the ticket recommendation.** A new structural
handler intercepts `pull_request.opened` events **before** the LLM turn (same pattern as
`ci_success_handler` / `ci_failure_handler` / `milestone_context_handler`) and removes `ready`
from each closing-issue of a newly-opened **draft** PR. Webhook-driven → immediate (vs the
10-min reconciler cadence of Option B), and cleanly separated from the auto-pull poller.

### Grounding facts established during investigation

- **Routing.** The gateway routes `pull_request.opened` (including draft opens) to **mika-qa**
  (`crates/mika-gateway/src/github.rs` `route_event`; mika#1822 confirms draft PRs reach mika-qa).
  The new handler lives in `server::handlers::handle_message`, which runs for **every**
  github-channel message regardless of target agent, and self-selects on the `[GitHub] PR opened:`
  text prefix — so it fires in mika-qa's session. It needs only a resolved `github_token`
  (mika-qa carries one), not any mika-dev-specific state.
- **Webhook text lacks the needed fields.** `format_event_text` for `pull_request`
  (`github.rs:442`) emits `[GitHub] PR opened: {repo}#{n} — {title} (branch: {branch})` and does
  **not** carry `isDraft` or `closingIssuesReferences`. The handler must fetch them with one
  `gh pr view` call — exactly the pattern `ci_success_handler` already uses (`run_gh_subprocess`).
- **Stable after removal (no thrash).** Once a draft PR is open and its `closingIssuesReferences`
  is populated, Phase 2's Filter 2 (`open_pr_closing`) already excludes that issue from re-drive,
  so the reconciler will **not** re-add `ready` after this handler removes it. The two mechanisms
  compose; no coordination flag is needed. (The ticket's "reconciler re-adds" observation was the
  *pre-draft* window — Phase 2 ran before the draft existed. Post-draft, Filter 2 holds.)
- **Reusable idempotent primitive.** `auto_pull.rs::gh_remove_label` (mika#1824) already implements
  an idempotent `gh issue edit --remove-label` that tolerates "label not present" as success — but
  it is `DEFAULT_REPO`-hardcoded and private to `auto_pull`. The new handler is repo-general (parses
  the repo from the webhook), so it uses `run_gh_subprocess` directly rather than that helper.

---

## Requirements

- **R1** — A new `server::draft_pr_opened_handler::try_handle_draft_pr_opened` intercepts
  `pull_request.opened` webhooks before the LLM turn. Self-selects on the `[GitHub] PR opened:`
  text prefix; passthrough for every other event type. Returns `VerdictAction::Passthrough`
  always (side-effect-only injector — never replaces the turn, like `milestone_context_handler`).
- **R2** — On a matching event, fetch `isDraft` + `closingIssuesReferences` via one
  `gh pr view <n> --repo <repo> --json isDraft,closingIssuesReferences` call.
- **R3** — Fire the cleanup **only** when `isDraft == true` **and** `closingIssuesReferences` is
  non-empty (AC1). Non-draft opens and draft opens with no closing-refs are no-ops (AC5b, AC5c).
- **R4** — For each referenced closing issue, if the `ready` label is present, remove it via
  `gh issue edit <ref> --repo <repo> --remove-label ready`. Idempotent — removing an absent label
  is a no-op (AC2).
- **R5** — Emit a structured `ready_label_cleaned_up_on_draft_open` INFO event per removal, with the
  repo, PR number, and issue number (AC3).
- **R6** — Fail-open on every `gh` error (missing token, `pr view` failure, per-issue `issue view` /
  `issue edit` failure): log WARN and continue; never crash the handler or the turn (AC4).
- **R7** — The repo is taken from the parsed webhook text, so the handler works for `mika`,
  `mika-cloud`, `mika-skills`, etc. — not hardcoded to `senara-solutions/mika`.

---

## Design decisions

### D1 — Handler shape: `VerdictAction::Passthrough`-only, side-effect injector

The handler mutates GitHub state (removes a label) as a pure side effect and does **not** need to
alter the mika-qa review turn. It returns `VerdictAction::Passthrough { enrichment: None }` on every
path — mirroring `milestone_context_handler`, which never returns `Handled`/`Dispatched`. Wiring in
`handlers.rs` matches only the two `Passthrough` arms and marks `Handled`/`Dispatched` as
`unreachable!` (same as the milestone handler's match block at `handlers.rs:998`).

Reusing `VerdictAction` (rather than a bespoke return type) keeps the handler uniform with the four
existing structural handlers and lets the wiring block stay copy-shaped.

### D2 — Accurate "per removal" semantics: check label presence before removing

AC3 requires an event **per removal**. Unconditionally calling `--remove-label` (which is
idempotent) would either over-emit (event on no-op removals) or under-inform. So for each closing
ref the handler first reads the issue's current labels
(`gh issue view <ref> --repo <repo> --json labels -q '.labels[].name'`), and only when `ready` is
present does it call `--remove-label` and emit `ready_label_cleaned_up_on_draft_open`. This makes
AC2 (idempotent) and AC3 (event-per-removal) both exact, at a cost of one extra read per closing
ref (closing refs are typically 1 per PR).

### D3 — Fetch fields via `gh`, not from the webhook (unavoidable)

`isDraft` and `closingIssuesReferences` are not in the gateway's PR event text (Context). The
handler issues `gh pr view <n> --repo <repo> --json isDraft,closingIssuesReferences` through the
shared `crate::tools::pr_merge_with_gate::run_gh_subprocess` (the same 60s-timeout subprocess
wrapper the sibling handlers use). Fail-open: any error → WARN + passthrough, no cleanup attempted.

### D4 — Pure decision seams for unit-testability (AC5)

Two pure functions carry the branching logic so AC5 is testable with **no network/subprocess**,
matching how `ci_success_handler` unit-tests its parser + formatters and explicitly leaves the live
subprocess path out of unit scope:

```rust
/// Parse `[GitHub] PR opened: <repo>#<n> — <title> (branch: <b>)` → (repo, number).
/// Returns None for any non-`PR opened` event text.
fn parse_pr_opened(text: &str) -> Option<PrOpenedEvent>;

/// The closing-issue numbers eligible for cleanup: `[]` unless `is_draft` AND refs non-empty.
fn plan_ready_label_cleanup(is_draft: bool, closing_issue_numbers: &[u64]) -> Vec<u64>;

/// True when the issue's current label set contains `ready` (case-sensitive, matches taxonomy).
fn issue_has_ready_label(labels: &[String]) -> bool;
```

The async `try_handle_draft_pr_opened` wires real `gh`/JSON into these predicates. AC5 (a/b/c) is
covered directly by `plan_ready_label_cleanup` + `issue_has_ready_label`; AC5(d) is covered by the
removal loop's per-issue fail-open structure (each `gh` call wrapped in match → WARN + continue),
asserted at the seam level plus a JSON-parse-tolerance test. The subprocess boundary itself is
excluded from unit scope, consistent with the existing handler tests.

### D5 — Draft-only, per the ACs

AC1 and AC5(b) restrict the trigger to `isDraft == true`; a non-draft open is an explicit no-op.
(Rationale: draft opens are the systematic dispatch-lib wip-rescue shape that produces the leftover
label; a non-draft open is a rarer, human-driven path. Widening to non-draft is out of scope — see
Out of scope.)

---

## Implementation steps

1. **New module** `crates/mika-agent/src/server/draft_pr_opened_handler.rs`. Declare `pub mod
   draft_pr_opened_handler;` in `server/mod.rs` (alongside the other handler mods).
2. **Parser** (`parse_pr_opened`, D4): strip `[GitHub] PR opened: `, take the token up to ` — `,
   `rsplit_once('#')` into `(repo, number)`, parse `number: u64`. Reject empty repo / unparsable
   number. Unit tests for owner/repo form, mika-cloud form, and rejection of `PR closed` / non-PR
   text.
3. **Pure decision fns** (`plan_ready_label_cleanup`, `issue_has_ready_label`, D4) + unit tests
   (AC5 a/b/c).
4. **Async handler** `try_handle_draft_pr_opened(text, github_token, session_id, trace_id, db?)`:
   - `parse_pr_opened` → passthrough on miss.
   - token check → WARN + passthrough on `None`.
   - `gh pr view` for `isDraft` + `closingIssuesReferences` → WARN + passthrough on error/parse fail.
   - `plan_ready_label_cleanup(is_draft, &refs)` → if empty, passthrough (DEBUG "no-op").
   - For each ref: `gh issue view --json labels`; if `issue_has_ready_label` → `gh issue edit
     --remove-label ready` + emit `ready_label_cleaned_up_on_draft_open` INFO (R5). Every `gh` call
     fail-open (WARN + continue) (R6).
   - Return `VerdictAction::Passthrough { enrichment: None }` (D1).
5. **Wiring** (`handlers.rs`, inside `if req.channel == "github"`, after the `milestone_context`
   block): resolve the same `verdict_github_token`, call `try_handle_draft_pr_opened`, match
   `Passthrough { enrichment }` (Some → prepend; None → no-op), `Handled`/`Dispatched` →
   `unreachable!`. Place it independent of the other handlers (self-selecting on event type).
6. **Audit event (optional, if `db` threaded):** write a `draft_pr_ready_label_cleaned` audit row
   per removal for operator-visible history, mirroring `ready_label_handler`'s `log_audit_event`
   use. Fire-and-forget (WARN on DB error). *(Include only if the wiring already has `&a.db` in
   scope — it does; keep it non-fatal.)*
7. **Tests** (AC5): parser tests; `plan_ready_label_cleanup` truth table (draft+refs → refs;
   non-draft+refs → []; draft+[] → []); `issue_has_ready_label` present/absent; a `gh pr view` JSON
   parse test over a captured fixture (isDraft + closingIssuesReferences array).

---

## Verification contract

- `cargo test -p mika-agent draft_pr_opened` — new parser + decision-seam tests green.
- `cargo clippy -p mika-agent -- -D warnings`, `cargo fmt --check`.
- The pure seams (`parse_pr_opened`, `plan_ready_label_cleanup`, `issue_has_ready_label`) fully cover
  AC5 (a/b/c) with no network/subprocess; the removal loop's fail-open structure covers AC5(d).
- No change to any existing handler's observable behavior (each self-selects on disjoint event
  types); existing `handlers.rs` handler tests unchanged and passing.

### Post-deploy verification (operator, AC6)

- Open a test draft PR whose body has `Closes #<n>` for an issue carrying `ready`. Within ~5s
  (webhook latency) confirm the label is gone (`gh issue view <n> --json labels`) and a
  `ready_label_cleaned_up_on_draft_open` line is present:
  `grep ready_label_cleaned_up_on_draft_open $MIKA_SPIRIT_LOG_FILE | jq 'select(.issue==<n>)'`.
- Steady-state: the count of `ready`-labelled issues that also have an open closing PR should trend
  to 0.

---

## Definition of Done

- `try_handle_draft_pr_opened` fires on `pull_request.opened` with `isDraft=true` +
  non-empty closing refs; removes `ready` from each closing issue that has it (R1–R5, AC1).
- Idempotent — absent label is a no-op; no event emitted when nothing removed (R4, D2, AC2).
- `ready_label_cleaned_up_on_draft_open` INFO emitted per removal (R5, AC3).
- Fail-open on all `gh`/token errors; handler and turn never crash (R6, AC4).
- Handler is repo-general (parses repo from the webhook) (R7).
- Unit tests pass (AC5); clippy/fmt clean; no regression to existing structural handlers.
- Wired into `handlers.rs` `handle_message`, order-independent with the four existing handlers.

---

## Acceptance criteria

- AC1: New handler fires on `pull_request.opened` with `isDraft=true` AND `closingIssuesReferences` non-empty. For each referenced issue, if `ready` label present, remove it.
- AC2: Handler is idempotent (removing an absent label is a no-op).
- AC3: Handler emits `ready_label_cleaned_up_on_draft_open` structured INFO event per removal.
- AC4: Handler fail-opens on gh CLI errors (warn log + continue).
- AC5: Unit tests cover: (a) draft PR opens with closing-refs → label removed; (b) non-draft PR opens → no-op; (c) draft with no closing-refs → no-op; (d) gh CLI failure → warn but no crash.
- AC6: Post-deploy verification — open a test draft PR closing an issue with `ready` label. Confirm label removed within 5s + structured log event emitted.

---

## Out of scope

- **Existing backlog cleanup.** Option A is webhook-driven — it fires only on **new**
  `pull_request.opened` events. The 5 issues already in the stuck state (their draft PRs opened in
  the past) will **not** be cleaned by this handler. They need a one-time operator sweep
  (`gh issue edit <n> --remove-label ready`), which is now stable because Phase 2 Filter 2 already
  refuses to re-add `ready` while an open closing-PR exists (Context). A reconciler-side proactive
  removal (the ticket's Option B) is a possible follow-up if backlog cleanup should be automated;
  not needed for AC1–AC6.
- **Non-draft PR opens.** The ACs scope the trigger to `isDraft=true` (D5); leftover `ready` on
  non-draft opens is theoretically possible but out of scope here.
- **`ready_for_review` (draft→ready promotion) events.** The label should already be gone by the
  time a draft is promoted; no handler needed on that transition.
- **Any change to the auto-pull poller (Option B).** Deliberately not touched — Option A chosen for
  separation and immediacy.

---

## Risks

- **R-routing** — the handler runs in mika-qa's session (that's where `pull_request.opened` lands).
  It performs only issue-label mutation via `github_token`, agent-agnostic. Mitigated: no
  mika-dev-specific state is read; the qa-review turn is unperturbed (Passthrough-only).
- **R-closing-refs-lag** — GitHub may populate `closingIssuesReferences` slightly after the
  `opened` event fires. If empty at handler time, the cleanup no-ops and the label lingers until a
  later signal. Mitigated: the leftover is bounded and Phase 2 Filter 2 keeps it stable (no
  re-add); backlog sweep (out of scope) covers the residue.
- **R-extra-reads** — one `gh issue view` per closing ref (D2). Bounded (≈1 ref/PR); acceptable for
  accurate per-removal event semantics.

## References

- mika#1824 (Phase 2 stuck-ready reconciler — Filter 2 skips, doesn't clean;
  `gh_remove_label` idempotent primitive).
- mika#1822 (draft PR `opened`/`ready_for_review` routing to mika-qa).
- mika#1282 (dispatch-lib wip-rescue draft-PR path — the systematic producer of the leftover label).
- Sibling structural handlers: `ci_success_handler`, `ci_failure_handler`, `milestone_context_handler`.
- Founding diagnostic: Samidarko-CC spool
  `2026-07-27-...engine-FROZEN-7h-restart-urgent-plus-label-bug-real.md`.
