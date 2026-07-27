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

### Phase 1: New Module — `wip_resume.rs` (~400-600 lines)

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
   - Step 1: **RESCUE_DEPTH guard** — fetch parent task metadata, read `wip_rescue.depth`. If ≥ max → bail-to-human (label `human-review-required`, comment, return)
   - Step 2: **Dry-run rebase** — `git merge-tree --write-tree main HEAD` on branch. If conflict → bail-to-human (comment "rebase-conflict-on-main")
   - Step 3: **Live rebase** — fast-forward or 3-way (no interactive). Fail → bail
   - Step 4: **Clippy 2-pass** — `cargo clippy --tests` on worktree. Errors → dispatch mika-dev rescue prompt. Re-clippy after fix. Still errors → bail
   - Step 5: **Re-run pilot** — dispatch dev-pilot on branch (reuse plan from issue body / existing worktree). Backoff-exp max 3 on transient errors
   - Step 6: **Substrate-diff check** — `perimeter::fetch::fetch_pr_files()` + `perimeter::classify_pr_files()`. DECISION-CORE → un-draft + comment "ready-for-Vincent-review" (do NOT auto-merge). MECHANICAL → proceed to un-draft
   - Step 7: **Un-draft** — `gh pr ready <n>` → triggers `pull_request.ready_for_review` webhook → mika-qa reviews → verdict → forge-gate → merge OR gate
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
- `gh pr ready` has no existing Rust implementation — needs new subprocess call pattern
- Git merge-tree dry-run rebase requires git 2.38+ (verify on deploy target)
- Clippy 2-pass dispatches mika-dev which may conflict with active sprint — concurrency cap mitigates
- RESCUE_DEPTH stored in parent task metadata — need to identify parent task from PR metadata
