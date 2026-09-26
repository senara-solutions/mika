# Plan: mika#1852 — Auto-Resume WIP-Rescue Drafts

## Ticket
mika#1852 — feat(dispatch): auto-resume wip-rescue drafts — rebase/fix-clippy/re-run-pilot/un-draft with substrate-diff human-gate (RT#004)

## Branch
`feat/1852/dispatch-auto-resume-wip-rescue-drafts`

## Architecture Research (from claude-pilot session 27e3b174)

### Existing Machinery to Reuse
1. **Perimeter classifier** (`crates/mika-agent/src/perimeter/`): `fetch_pr_files()`, `classify_pr_files()`, `Classification::{Mechanical, DecisionCore}` — READY from mika#1831
2. **Verdict handler perimeter check** (`verdict_handler.rs:294-326`): already hoisted before task lookup (mika#1853)
3. **Auto-pull cron pattern** (`auto_pull.rs`): `ensure_recurring_task` registration at server startup, cron-based scan, `log_audit_event`
4. **Dispatch-lib wip-rescue creation** (`dispatch-lib.sh:2549-2667`): creates draft PRs with `wip-rescue` label, `RECOVERY_PENDING: true` metadata marker
5. **Task metadata** (`async_db.rs:2171`): `update_task_metadata()` with shallow JSON merge via `merge_metadata()`
6. **Per-class dispatch slot** (`executor.rs`): one `implement` + one `groom` concurrent per agent
7. **Deferred dispatch** (mika#1070): `DeferredDispatch` silent turns, `promote_pending_deferred_if_idle` backstop
8. **Worktree cleanup** (mika#1417): reaper machinery — reuse, don't reinvent
9. **wip-staleness-check** (`.github/workflows/wip-staleness-check.yml`): GitHub Actions probes `wip-rescue` label + `wip(` title prefix, applies `stale-against-main` label
10. **GitHub ops**: all via `gh cli` subprocess (no octocrab SDK), `spawn_and_collect()` wrapper

### Key Findings
- **No `gh pr ready` in Rust yet** — needs new builtin or skill handler
- **No octocrab** — all GitHub interaction via `gh` CLI subprocess
- **Perimeter fail-closed**: empty file list → `DecisionCore`, fetch error → `DecisionCore`
- **Dispatch class**: dev-pilot → `implement`, dev-groom → `groom`; concurrency = 1 per class per agent
- **RESCUE_DEPTH counter**: persisted in parent task metadata `$.wip_rescue.depth`

## Implementation Plan

### Phase 1: New Module — `wip_rescue.rs` (~400-600 lines)

**File:** `crates/mika-agent/src/wip_rescue.rs`

**Contents:**

1. **Env var parsing** (pure function, testable):
   - `MIKA_WIP_RESCUE_MIN_AGE_SECS` — default 900 (15min)
   - `MIKA_WIP_RESCUE_MAX_DEPTH` — default 2
   - `MIKA_WIP_RESCUE_CRON` — default `"0 */5 * * * *"` (every 5 min)
   - Three-tier fallback: empty/absent → default, unparseable/negative → WARN + default, valid → use value

2. **Main orchestrator**:
   ```rust
   pub async fn auto_resume_wip_rescue_drafts(
       db: &AsyncDb,
       github_token: &str,
       trace_id: &str,
       session_id: &str,
   ) -> Option<usize>
   ```
   - Lists open draft PRs with `wip-rescue` label via `gh pr list --draft --json number,headRefName,title,labels,createdAt`
   - Filters by age > threshold
   - For each: runs `rescue_chain()` with global concurrency cap of 1
   - Returns count of successfully resumed drafts

3. **`resume_chain()` — per-draft cost-bounded chain**:
   - Step 1: **RESCUE_DEPTH guard** — resolve the parent task, fetch its metadata, read `wip_rescue.depth`. If ≥ max → bail-to-human (label `human-review-required`, comment, return).
     - **Parent-task resolution chain (F2):** map PR → closing issue → task row.
       1. `gh pr view <n> --json closingIssuesReferences` yields the closing issue number(s). wip-rescue drafts always carry a single `Closes #<issue>` (created by dispatch-lib), so take the first.
       2. Query the task table by that issue's `reference_url` — the existing `get_task_by_reference_url()` / equivalent async_db lookup keyed on the issue URL (same key dispatch-lib and auto_pull already use to correlate a ticket to its `tasks` row). This is the reuse path called out in the F2 citation (DRY): no new join table, no PR-description metadata parsing.
       3. If no task row is found (e.g., the draft predates task creation or the issue link is missing), treat depth as `0` for this attempt and emit `wip_rescue_skipped` with reason `"no_parent_task"` so the absence is observable — but still proceed (the draft is real work; a missing ledger row must not strand it). The depth counter then lives on whatever task row the re-run pilot dispatch creates/updates.
     - The `wip_rescue.depth` counter is persisted in that parent task's metadata under `$.wip_rescue.depth` via the existing `update_task_metadata()` shallow-merge (`async_db.rs:2171`).
   - Step 2: **Dry-run rebase** — `git merge-tree --write-tree main HEAD` on branch. If conflict → bail-to-human (comment "rebase-conflict-on-main").
     - **Git version guard (F3, fail-closed):** `git merge-tree --write-tree` requires git ≥ 2.38. At the *start* of `resume_chain()` (before any mutation), run a one-shot `git version_supports_writetree()` check that parses `git --version` and compares against 2.38.0. If the version is unknown/older, do NOT silently fall back to a mutating rebase (a fallback that skips the dry-run would violate the "never mutate before a clean dry-run" safety invariant this feature exists to enforce). Instead **fail closed**: call `bail_to_human()` with reason `"git-too-old-for-dry-run:<version>"` and emit `wip_rescue_error`. This is a deploy-environment defect, not a per-PR condition, so bailing to human is the correct escalation (Citation: review-guide.md § Fail-Closed). The check result can be memoized per process (version does not change mid-run).
   - Step 3: **Live rebase** — fast-forward or 3-way (no interactive). Fail → bail
   - Step 4: **Clippy 2-pass** — `cargo clippy --tests` on worktree. Errors → dispatch mika-dev rescue prompt. Re-clippy after fix. Still errors → bail.
     - **Dispatch priority (F4):** the mika-dev clippy-fix dispatch is issued at **low priority (`priority: rescue`, fond-de-file)** — never normal priority. It must not preempt mission-avant sprint work in the dispatch queue (Citation: RT#004 "lower priority than mission-avant work"). Concretely: the dispatch inherits the same low-priority scheduling as the parent `wip_rescue` cron action (Phase 3, AC7), and the global concurrency cap of 1 (AC6) already bounds how many rescue dispatches can be in-flight. If mika-dev's `implement` slot is occupied by active sprint work, the rescue clippy-fix waits for the slot rather than jumping it. Priority annotation is carried on the dispatched task metadata (`$.priority = "rescue"`) so the executor's slot arbitration deprioritizes it and it is greppable in telemetry.
   - Step 5: **Re-run pilot** — dispatch dev-pilot on branch (reuse plan from issue body / existing worktree). Backoff-exp max 3 on transient errors
   - Step 6: **Substrate-diff check** — `perimeter::fetch::fetch_pr_files()` + `perimeter::classify_pr_files()`. DECISION-CORE → un-draft + comment "ready-for-Vincent-review" (do NOT auto-merge). MECHANICAL → proceed to un-draft
   - Step 7: **Un-draft** — explicit subprocess call (F1). There is no `gh pr ready` Rust builtin; the un-draft is a direct `gh` CLI subprocess through the existing `spawn_and_collect()` wrapper (same pattern as `gh pr list` in the orchestrator). Concrete invocation:
     ```rust
     // reuse the token-injecting subprocess wrapper used elsewhere in the module;
     // args: ["pr", "ready", &pr_number.to_string(), "--repo", &repo]
     let out = run_gh_subprocess(
         &["pr", "ready", &pr_number.to_string(), "--repo", repo],
         github_token,
         Duration::from_secs(30), // per-call timeout, same as other gh calls
     ).await;
     ```
     - **Timeout:** 30s (matches the other `gh` calls in the chain).
     - **Fail-open / bail-to-human on failure:** if the subprocess exits non-zero or times out, do NOT retry the un-draft and do NOT leave the PR in a half-resumed state — call `bail_to_human()` with reason `"un-draft-failed:<stderr-snippet>"`. The draft stays a draft (safe default: work is preserved, human decides). Emit `wip_rescue_error` with the `gh` stderr.
     - On success, `gh pr ready` triggers the `pull_request.ready_for_review` webhook → mika-qa reviews → verdict → forge-gate → merge OR gate. No further action from this chain.
   - Increment `wip_rescue.depth` on each attempt

4. **Bail-to-human**:
   - Add label `human-review-required` via `gh pr edit --add-label`
   - Post PR comment naming bail reason via `gh pr comment`
   - Emit `wip_rescue_bail_to_human` structured event with reason
   - End chain — NO further auto-attempts

5. **Observability** (structured tracing events):
   - `wip_rescue_resume_attempt` — INFO, fields: `pr_number`, `age_secs`, `rescue_depth`
   - `wip_rescue_bail_to_human` — WARN, fields: `pr_number`, `reason`
   - `wip_rescue_success` — INFO, fields: `pr_number`, `classification`
   - `wip_rescue_skipped` — DEBUG, fields: `reason`
   - `wip_rescue_error` — WARN, fields: `pr_number`, `error`
   - `log_audit_event()` for each resume action

### Phase 2: Server Registration (~5 lines)

**File:** `crates/mika-agent/src/server/mod.rs`

- Add cron const: `const WIP_RESCUE_CRON: &str = "0 */5 * * * *";`
- Register at startup: `ensure_recurring_task(&db, "wip_rescue_drafts", WIP_RESCUE_CRON, r#"{"trigger":"wip_rescue"}"#).await;`
- Add `pub mod wip_rescue;` to `lib.rs` or `mod.rs`

### Phase 3: Dispatcher Action (~15 lines)

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

- Match: `"wip_rescue" => Ok(self.dispatch_wip_rescue(task).await?)`
- Implement `dispatch_wip_rescue()` following `dispatch_auto_pull_groomed()` pattern (line 854)
- Low-priority scheduling — does not preempt normal dispatch

### Phase 4: Tests

- Unit tests for env-var threshold parse function (absent, empty, valid, invalid, negative)
- Unit tests for age filter logic
- Unit tests for RESCUE_DEPTH guard (depth 0, 1, 2, 3)
- Integration test: mock `gh pr list` response, verify chain execution
- Integration test: verify bail-to-human on merge-tree conflict
- Integration test: verify perimeter classification routing (MECHANICAL → un-draft, DECISION-CORE → un-draft + comment)
- Post-deploy verification (AC9): create test wip-rescue draft, wait for scan, verify outcome

## AC Mapping

| AC | Implementation |
|----|----------------|
| AC1 | Env var `MIKA_WIP_RESCUE_MIN_AGE_SECS` (default 900), age filter in `auto_resume_wip_rescue_drafts()` |
| AC2 | `resume_chain()` — 7-step chain with RESCUE_DEPTH gate |
| AC3 | `bail_to_human()` — label + comment + structured event |
| AC4 | Reuses `perimeter::fetch::fetch_pr_files` + `perimeter::classify_pr_files` from mika#1831 |
| AC5 | NO trivial-diff carve-out — all drafts go through perimeter classifier |
| AC6 | Global concurrency cap = 1 (checked before entering chain) |
| AC7 | Low-priority scheduling (cron-driven, does not preempt normal dispatch) |
| AC8 | 5 structured tracing events + `log_audit_event()` per action |
| AC9 | Post-deploy: create test draft, verify scan picks it up |

## Out of Scope (per ticket)
- Trivial-diff auto-merge carve-out (AC5 STRICT — YAGNI)
- Non-wip-rescue draft auto-resume
- Rebase-conflict auto-resolution beyond fast-forward + 3-way
- claude-pilot-py policy fix (F3 — separate PR, gated)

## Definition of Done
- `wip_rescue.rs` module implements the full 7-step cost-bounded chain with RESCUE_DEPTH gate.
- Server registers the periodic scan cron; dispatcher routes the `wip_rescue` trigger to a low-priority action.
- Global concurrency cap of 1 auto-resume in-flight is enforced before entering any chain.
- Perimeter classification (mika#1831) is reused verbatim — no reimplementation of the DECISION-CORE/MECHANICAL classifier.
- All five structured events (`wip_rescue_resume_attempt`, `wip_rescue_bail_to_human`, `wip_rescue_success`, `wip_rescue_skipped`, `wip_rescue_error`) are emitted with the documented fields, plus `log_audit_event()` per action.
- Unit + integration tests cover env-var parsing, age filter, RESCUE_DEPTH guard, bail-to-human, and perimeter routing.
- `cargo build`, `cargo clippy`, `cargo test`, and `cargo fmt --check` all pass.
- Post-deploy AC9 verification performed (test wip-rescue draft resumed or bailed with `human-review-required`).

## Acceptance criteria
- **AC1** — Periodic scan identifies wip-rescue drafts age > threshold (env `MIKA_WIP_RESCUE_MIN_AGE_SECS`, default 900).
- **AC2** — Per draft: rebase dry-run → live rebase → clippy 2-pass → pilot re-run → substrate-diff check → un-draft chain, with RESCUE_DEPTH gate bailing to human at max 2.
- **AC3** — Bail-to-human: adds label `human-review-required`, posts PR comment naming the bail reason, ends chain. NO further auto-attempts.
- **AC4** — Substrate-diff check reuses `perimeter::fetch::fetch_pr_files` + `perimeter::classify_pr_files` (from mika#1831). DECISION-CORE → un-draft + comment "Vincent-hand-merge" (do not run verdict merge). MECHANICAL → un-draft normally (verdict handler will invoke merge).
- **AC5 STRICT** — NO trivial-diff auto-merge carve-out. Even 1-line diffs go through forge-gate.
- **AC6** — Concurrency cap: max 1 auto-resume attempt in-flight globally. Excess wait for next scan tick.
- **AC7** — Priority: uses low-priority scheduling (fond-de-file), does not preempt normal dispatch.
- **AC8** — Observability: emits `wip_rescue_resume_attempt`, `wip_rescue_bail_to_human` (with reason), `wip_rescue_success` structured events for grep/dashboard.
- **AC9** — Post-deploy verification: create test wip-rescue draft (mechanically stale), wait for scan; expect either successful un-draft + qa-review → merge, OR bail with human-review-required label + comment.

## Files Modified/Created

| File | Action | Lines |
|------|--------|-------|
| `crates/mika-agent/src/wip_rescue.rs` | NEW | ~400-600 |
| `crates/mika-agent/src/server/mod.rs` | MODIFY | ~5 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | MODIFY | ~15 |
| `crates/mika-agent/src/lib.rs` or `mod.rs` | MODIFY | 1 |

## Dependencies
- mika#1831 (forge-gate perimeter) — MERGED ✓
- mika#1853 (perimeter bypass fix) — MERGED ✓
- mika#1417 (worktree cleanup) — READY ✓
- mika#1850 (liveness heartbeat) — MERGED ✓ (alert-only, no auto-restart)

## Risk Notes
- `gh pr ready` has no existing Rust implementation — resolved (F1): explicit `run_gh_subprocess(["pr", "ready", ...])` call with 30s timeout + bail-to-human on failure, specified in Step 7.
- Git merge-tree dry-run rebase requires git 2.38+ — resolved (F3): fail-closed version guard at chain start (`git-too-old-for-dry-run` bail), no silent mutating fallback.
- Clippy 2-pass dispatches mika-dev which may conflict with active sprint — resolved (F4): rescue dispatch pinned to low priority (`$.priority = "rescue"`, fond-de-file) + AC6 concurrency cap; never preempts sprint work.
- RESCUE_DEPTH stored in parent task metadata — resolved (F2): parent task resolved via PR → `closingIssuesReferences` → task lookup by issue `reference_url` (reuse of existing correlation key); depth defaults to 0 with `wip_rescue_skipped:no_parent_task` when no row exists.

## Revision history
- rev 2 (2026-07-27): addressed F1 by specifying the explicit `gh pr ready` subprocess call in Step 7 (invocation, 30s timeout, fail-open→bail-to-human); addressed F2 by defining the parent-task resolution chain (PR → closing issue → task lookup by `reference_url`, reusing the existing correlation key, with a `no_parent_task` observable-skip fallback); addressed F3 by adding a fail-closed git ≥ 2.38 version guard at chain start (bail `git-too-old-for-dry-run`, no silent mutating fallback per § Fail-Closed); addressed F4 by pinning the mika-dev clippy-fix dispatch to low priority (`$.priority = "rescue"`, fond-de-file) so it never preempts mission-avant sprint work (RT#004). Risk Notes updated to mark all four resolved.
