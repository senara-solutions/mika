---
title: "fix(engine): pr_merge_with_gate writes pr_url to supervisor on auto_merge_enabled — prevent reaper false-positive (mika#1211)"
type: fix
status: active
date: 2026-05-20
---

# fix(engine): pr_merge_with_gate writes pr_url to supervisor on auto_merge_enabled

## Overview

When `pr_merge_with_gate` returns `auto_merge_enabled` (CI pending, PR queued for GitHub auto-merge), today's tool path does not record the PR URL on the supervisor (manual-parent) task's metadata. If `try_extract_callback_metadata` also failed to populate `$.claude_pilot.pr_url` from the dispatch-lib `PR:` line (race, missing branch, gh CLI hiccup, qa-only callback delivery, etc.), the supervisor sits `in_progress` with `pr_url IS NULL`. Once the dispatch callback child has been `delivered` longer than `REAPER_GRACE_SECONDS` (600s), the next `reap_orphaned_parent_tasks` tick (every 60s) sees:

- `parent.status = 'in_progress'`, `parent.source = 'self_dev'`, `parent.trigger_type = 'manual'` ✓
- `child.status = 'delivered'`, `child.dispatch_class = 'implement'`, `child.updated_at < now-600s` ✓
- `parent.metadata.$.claude_pilot.pr_url IS NULL` ✓
- `NOT EXISTS active sibling` ✓

…and writes `tasks.result = 'callback_delivered_without_pr_url'`, transitioning the supervisor to `status = 'failed'` without any tool call. This produces the misleading state described in the issue: `update_task_status` flipped the supervisor `blocked → in_progress` at 09:41:54Z; the reaper flipped it `in_progress → failed` at 09:42:09Z; PR #1206 auto-merged later, the issue closed cleanly, but the supervisor remained `failed`.

This fix has `pr_merge_with_gate` itself write `$.claude_pilot.pr_url` to the supervisor's metadata when it successfully enables auto-merge. The PR URL is constructed deterministically from the tool's `pr_number` and `repo` inputs. Once `pr_url` is present, both backstops behave correctly:

- The reaper skips the parent (its predicate is `pr_url IS NULL`).
- The parent-completer (`complete_parent_tasks_on_callback_success`, mika#1162) sees `pr_url IS NOT NULL` and, after the same 600s grace, transitions the parent `in_progress → completed`. This matches the existing semantic of the `merged` path: pr_url presence = "a PR was produced; the supervisor's work is done."

The fix is structurally identical to a metadata-write the tool already implies — it just makes the implication explicit. No new schema, no new metadata field, no prompt-level change required of mika-dev.

## Problem Frame

**Canonical incident (issue body, mika#1211, 2026-05-19 mika#1204 dispatch):**

- Tool call `0872cc14` at 09:41:46Z: `pr_merge_with_gate → auto_merge_enabled` for PR #1206.
- Tool call `6004e470` at 09:41:54Z: `update_task_status` flips supervisor `8b15af11` `blocked → in_progress` with note "QA passed, auto-merge enabled, awaiting CI. PR: https://github.com/senara-solutions/mika/pull/1206".
- Supervisor `8b15af11` status at 09:42:09Z: `failed`. **No `update_task_status` tool call in `tool_calls` table between 09:41:54Z and 09:42:09Z** — the transition came from an engine-side writer.
- PR #1206 auto-merged later. Issue closed via the `Closes` reference. Functional success. Supervisor remained `failed`.

**Why the engine wrote `failed`.** Code reading (`crates/mika-agent/src/db.rs:5509-5551`, `crates/mika-agent/src/task_engine/engine.rs:638-803`) confirms the reaper is the only engine-side writer that transitions a manual `in_progress` self_dev task to `failed` with `tasks.result = 'callback_delivered_without_pr_url'`. The 15-second elapsed time between the `update_task_status` and the `failed` transition is consistent with the reaper's 60s tick cadence — the prior dispatch callback child had delivered earlier in the cycle (well outside the 600s grace), so the very next tick after the supervisor flipped back to `in_progress` matched the reaper's predicate set.

**Why `pr_url` wasn't already in metadata.** `try_extract_callback_metadata` (mika#376, `dispatcher.rs:1224`) populates `$.claude_pilot.pr_url` from the `PR: <url>` line emitted by `skills/bundled/_shared/dispatch-lib.sh:618-625` via `gh pr list --repo <repo> --head <branch>`. This emission depends on three runtime conditions (`$REPO`, `$BRANCH`, `gh` CLI success); any failure leaves `pr_url` unset. Additionally, the autonomous loop dispatches multiple successive callbacks per supervisor (dev-pilot → qa-review-build-callback → qa-review → potentially address-pr-comments / resolve-pr-conflicts). Only dev-pilot's dispatch-lib emits `PR:`; other callbacks deliver without it. If the earliest pr_url-bearing delivery raced with metadata writes from later callbacks, or hit any of the `PR:` emission's runtime preconditions, the supervisor's metadata can be `pr_url IS NULL` at the time `pr_merge_with_gate` fires.

**Why fixing only at the reaper is wrong.** Skipping the reaper based on a heuristic (e.g., "recently transitioned" or "note matches /auto.?merge/") leaves the supervisor in a state the engine cannot reason about. The reaper's contract is "callback delivered without pr_url means the supervisor's work didn't produce a PR." When `pr_merge_with_gate` returns `auto_merge_enabled`, the PR demonstrably exists — recording it cures the predicate at its semantic source. This is the same reason `try_extract_callback_metadata` writes pr_url from the `PR:` line: pr_url is the canonical "a PR was produced" signal, and any code path that knows a PR exists should keep it accurate.

## Requirements Trace

- **R1.** After `pr_merge_with_gate` returns `MergeGateResult::AutoMergeEnabled`, the supervisor task's metadata contains `$.claude_pilot.pr_url = "https://github.com/senara-solutions/<repo>/pull/<n>"`. (Issue body §Proposed Fix option (a); §Expected Behavior (a).)
- **R2.** The reaper (`reap_orphaned_parent_tasks`) does NOT transition the supervisor to `failed` while auto-merge is pending. The existing `parent.metadata.$.claude_pilot.pr_url IS NULL` predicate suffices once pr_url is written. (Issue body §Expected Behavior — the supervisor should stay `in_progress`.)
- **R3.** The parent-completer (`complete_parent_tasks_on_callback_success`, mika#1162) transitions the supervisor `in_progress → completed` on its next periodic pass once the callback child has been `delivered` longer than `REAPER_GRACE_SECONDS`. (Matches existing semantic of the `merged` path; consistent with Issue body §Expected Behavior (a).)
- **R4.** When `pr_merge_with_gate` is called outside a callback turn (conversation mode, mika-arch dispatch, etc.), the tool's behavior is unchanged from today. No metadata write is attempted because no supervisor is identifiable; the tool still returns the same `MergeGateResult`.
- **R5.** When `pr_merge_with_gate` is called inside a callback turn but the parent of the callback is not a manual `self_dev` supervisor (e.g., the callback's parent is a milestone/project parent, or the trigger_type is not `'manual'`), the tool MUST NOT mis-write metadata to an unrelated task. Skip silently in that case.
- **R6.** The fix recovers supervisors wedged by this exact failure mode BEFORE deploy only via the existing parent-completer once the operator manually clears the `failed` status — pre-existing wedged-as-failed supervisors are out of scope for retroactive recovery. (Forward-only fix; no schema migration or backfill task.)
- **R7.** No regression in existing tests for `pr_merge_with_gate` (`crates/mika-agent/src/tools/pr_merge_with_gate.rs` test module), the reaper (`engine.rs` reaper tests), or the parent-completer (`engine.rs` completer tests, `dispatcher.rs` inline-completer tests).

## Scope Boundaries

- **Out of scope: clearing pr_url if the auto-merge eventually fails (PR closed without merge).** The existing `merged`-path semantic also doesn't handle this — if a merged PR is later reverted or the issue is re-opened, the supervisor stays `completed`. Treating auto-merge symmetrically keeps the system consistent. A separate ticket can introduce an "auto-merge resolved against us" handler (PR closed without merge → mark parent `failed`) if operator forensics show it matters. The issue body lists this as expected behavior (b) but acknowledges the current parent-completer doesn't verify actual merge state either.
- **Out of scope: investigating WHY `try_extract_callback_metadata` missed pr_url in the mika#1204 incident.** Code reading shows multiple plausible runtime failure modes for `dispatch-lib.sh:618-625` (`gh pr list` returns empty, branch not yet visible, `$BRANCH` unset on intermediate handlers, multiple successive callbacks where only the first emits `PR:`). Naming the specific cause for mika#1204 would require recovering DB and dispatch-lib log state from 2026-05-19. The structural fix closes the gap for every such cause at once.
- **Out of scope: prompt-level changes to mika-dev's auto-merge handling.** The issue body's option (a) ("recognize 'auto-merge enabled' note as a heartbeat") is prompt-fragile. Today's `update_task_status` call includes the auto-merge fact in a `note` field as free text; tightening that to require a structured `auto_merge_pending` metadata key would couple the engine to mika-dev's prompt discipline. The tool-level write in this fix is structural — it fires regardless of what mika-dev writes.
- **Out of scope: changing the reaper or the parent-completer.** Both backstops already behave correctly for `pr_url IS NOT NULL` (skip / promote, respectively). The fix records the missing signal at its source.
- **Out of scope: `MergeGateResult::Merged` and `MergeGateResult::AlreadyMerged` paths.** In the `Merged` path, the supervisor is typically already promoted by `try_complete_parent_on_callback_success` (mika#1162) before the operator-callable merge happens, and `try_extract_callback_metadata` populated pr_url from the original dispatch. In the `AlreadyMerged` path, the supervisor is either already terminal or will be promoted on the next callback cycle. Both paths could benefit symmetrically from a metadata write, but neither is the canonical bug path for this ticket. Limiting the scope to `AutoMergeEnabled` keeps the patch surface minimal; symmetric extension is a clean follow-up if forensics show it matters.

## Context & Research

### Relevant code and patterns

- **`crates/mika-agent/src/tools/pr_merge_with_gate.rs:192-224`** — the `CheckClassification::HasPending` branch where `MergeGateResult::AutoMergeEnabled` is constructed. The metadata write goes here, after `run_gh_merge(..., auto_merge=true, ...)` returns Ok, before the `ToolOutput::success` return.
- **`crates/mika-agent/src/tools/pr_merge_with_gate.rs:85-145`** — the tool's `execute` body, where `pr_number`, `repo`, `merge_method`, `delete_branch` are parsed from input. The PR URL is constructed from `pr_number` + `repo`. The repo input is the bare name (e.g., `mika`); the canonical URL form is `https://github.com/senara-solutions/<repo>/pull/<n>`. Verify the repo input shape matches what `try_extract_callback_metadata` writes (it does — both use the GitHub-canonical owner/repo path).
- **`crates/mika-agent/src/tool_context.rs`** (or equivalent `ToolContext` definition site) — `callback_task_id: Option<&str>` is the field that identifies "this tool is running inside a callback turn." Reads via `ToolContext.callback_task_id`. The CLAUDE.md description in `crates/mika-agent/CLAUDE.md` § Tools confirms: `Some` for `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch` turns, `None` otherwise.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:1224-1283`** — `try_extract_callback_metadata`. Reference pattern for the two-level shallow metadata merge (`crate::task_metadata::merge_metadata`) and the fire-and-forget write via `db.update_task_metadata`. Our new write reuses this exact pattern — read the supervisor, build the patch object `{"claude_pilot": {"pr_url": "..."}}`, merge with existing metadata, persist via `update_task_metadata`.
- **`crates/mika-agent/src/db.rs:5509-5551`** — `find_orphaned_parent_tasks` (the reaper's query). The `pr_url IS NULL` predicate is the load-bearing line our fix neutralizes. No code change here — verifying the predicate semantics holds post-fix.
- **`crates/mika-agent/src/db.rs:5572-5610`** — `find_completable_parent_tasks_on_pr_url` (the parent-completer's query). Once pr_url is written, this query selects the supervisor on the next periodic pass and the parent-completer transitions it `in_progress → completed`.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:1393-1480`** (approx.) — `try_complete_parent_on_callback_success` and the inline counterpart that the parent-completer uses. Confirms the existing transition semantics for pr_url-present.
- **`crates/mika-agent/src/task_metadata.rs`** — `merge_metadata` helper. Two-level shallow merge with object-merge at level 1. Identical semantics to `update_task_status`'s metadata-merge, so the same shape `{"claude_pilot": {"pr_url": "..."}}` patch works correctly.

### Institutional learnings

- **`docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`** — codifies the pattern: engine-side metadata writes happen regardless of LLM behavior. This fix extends the principle from "callback-result-text parsing" to "tool-output-driven writes": when a tool deterministically knows a fact (a PR exists at this URL), the tool writes it.
- **`docs/solutions/architecture-patterns/callback-task-loop-prevention.md`** — the convention that engine-level handlers are sole-writers of specific transitions. This fix doesn't introduce a new transition writer; it makes the supervisor metadata accurate so the existing writers (reaper, parent-completer) behave correctly.
- **`docs/solutions/architecture-patterns/work-item-metadata-two-level-shallow-merge.md`** — confirms the merge semantics for `$.claude_pilot.<field>` keys. Our patch object is one level deep, fully aligned with this pattern. mika#489 / mika#617 lineage.
- **mika#871, #1126** (the reaper) and **mika#1162** (the parent-completer) — the coupled pair this fix relies on. The reaper's `pr_url IS NULL` and the completer's `pr_url IS NOT NULL` partition the world; this fix puts the supervisor on the correct side of that partition.

### Coupled pairs (from CLAUDE.md conventions)

- **`pr_merge_with_gate` and `try_extract_callback_metadata`** are now a coupled pair on the `$.claude_pilot.pr_url` key. Both write to the supervisor's metadata under the same JSON path. The merge semantics (two-level shallow merge via `merge_metadata`) guarantee they compose correctly regardless of order: whichever fires first writes pr_url; the second sees pr_url already present and is idempotent (re-writing the same URL). Any future change to the metadata schema for `pr_url` MUST update both writers symmetrically.
- **`pr_merge_with_gate` (this change) and the reaper / parent-completer (existing)** are not modified together, but the fix makes them behave correctly together. Future changes to the reaper's `pr_url IS NULL` predicate or the completer's `pr_url IS NOT NULL` predicate MUST consider that `pr_merge_with_gate` writes pr_url at auto-merge time.

## Key technical decisions

- **D1: Write pr_url inside the tool, not via a separate engine handler.** Alternatives considered: (a) have mika-dev's prompt write `auto_merge_pending` to metadata via `update_task_status`; (b) intercept `MergeGateResult::AutoMergeEnabled` in the agent loop and write metadata post-hoc; (c) write inside the tool. Option (a) is prompt-fragile (drift across model swaps). Option (b) requires the agent loop to know about specific tool outputs — a layering violation. Option (c) keeps the write at the moment of decision: the tool just produced the structured signal, the tool writes the structured fact. This is the same pattern `try_extract_callback_metadata` uses on the engine side, applied symmetrically on the tool side.
- **D2: Construct pr_url from inputs, not from gh CLI output.** `run_gh_merge` returns a non-deterministic textual output we'd have to parse. The tool already has `pr_number` and `repo`; the canonical URL is `https://github.com/senara-solutions/<repo>/pull/<n>`. This avoids a parsing dependency on gh CLI output format and matches what dispatch-lib emits for the same PR.
- **D3: Resolve the supervisor via `ToolContext.callback_task_id → parent_task_id`, gated by `trigger_type='manual'` and `source='self_dev'`.** This mirrors the reaper's filter set. If any guard fails (no callback context, parent is not manual, parent is not self_dev, parent.metadata.pr_url already present), skip silently — the tool still returns `ToolOutput::success` with the unchanged `MergeGateResult::AutoMergeEnabled`. The skip cases are: conversation mode (callback_task_id is None), milestone/project parents, mika-arch / orchestrator manual invocation, and the "already-written" idempotency case.
- **D4: Fire-and-forget metadata write.** Match `try_extract_callback_metadata`'s pattern: log on failure, do NOT propagate the error into the tool result. The merge gate's primary contract is "did we enable auto-merge or not?" — that fact is already established by the time we reach the metadata write. Metadata-write failure shouldn't cause mika-dev to see `gate_errored` and retry the gh CLI.
- **D5: Use the existing `crate::task_metadata::merge_metadata` helper, not raw JSON manipulation.** Same shallow-merge semantics as `try_extract_callback_metadata` and `update_task_status`. No risk of clobbering other `claude_pilot.*` fields (session_id, cost_usd, etc.) or top-level metadata keys.
- **D6: No new audit event from the tool's write.** The reaper and parent-completer emit `task_engine_reaper` / `task_engine_parent_completer` audit events on transitions; this fix changes only metadata, not status. Metadata writes are not audit-event events (consistent with `try_extract_callback_metadata`'s no-audit-event pattern). Structured log at `info!` level (`engine: persisted auto-merge pr_url to supervisor`) suffices.
- **D7: Apply ONLY to the `AutoMergeEnabled` branch in this ticket.** The `Merged` and `AlreadyMerged` paths arguably benefit from the same write (the supervisor could equivalently leak if pr_url was missed by `try_extract_callback_metadata`). Limiting to `AutoMergeEnabled` keeps the patch surface minimal and matches the issue's stated reproduction; symmetric extension to `Merged` / `AlreadyMerged` is a clean follow-up ticket if forensics surface a leaked supervisor in the merged path.
- **D8: No schema change, no new metadata field, no migration.** The fix writes an existing JSON path (`$.claude_pilot.pr_url`) with the same shape `try_extract_callback_metadata` already writes. Forward and backward compatibility are trivial.

## Implementation plan

### Phase 1 — Tool-side metadata write

1. **Resolve supervisor task in `pr_merge_with_gate.rs`.** In the `CheckClassification::HasPending` branch, AFTER `run_gh_merge(..., auto_merge=true, ...)` returns Ok and BEFORE constructing `MergeGateResult::AutoMergeEnabled`, attempt to resolve the supervisor task ID:
   - If `ctx.callback_task_id.is_none()`, skip the metadata write entirely (no supervisor identifiable; conversation mode or non-callback context).
   - If `Some(callback_id)`, fetch the callback task via `ctx.db.get_task_unscoped(callback_id)`. If it has no `parent_task_id`, skip. If the parent task does not satisfy `trigger_type == "manual" && source == Some("self_dev")`, skip (defense-in-depth: don't write to milestone/project parents or non-self_dev tasks).
2. **Construct the PR URL.** Format: `format!("https://github.com/senara-solutions/{}/pull/{}", repo, pr_number)`. Use the bare `repo` input verbatim. Validate that `repo` is non-empty (existing input validation should already ensure this; add a defensive check if not).
3. **Build the metadata patch.** Use `serde_json::json!({"claude_pilot": {"pr_url": pr_url}})` to construct the patch object. Read the supervisor's existing metadata; merge with `crate::task_metadata::merge_metadata`; persist via `ctx.db.update_task_metadata(supervisor_id, &merged.to_string())`.
4. **Logging.** On success: `info!(supervisor_task_id = %supervisor_id, pr_url = %pr_url, "pr_merge_with_gate: wrote pr_url to supervisor metadata on auto_merge_enabled")`. On any failure (DB error, JSON serialization error, supervisor not found): `warn!(...)` — do NOT propagate into the tool result.
5. **Return path unchanged.** After the metadata-write attempt (success or skip), continue to build `MergeGateResult::AutoMergeEnabled { pending_checks }` and return `ToolOutput::success(serde_json::to_string_pretty(&result)?)` as today.

**File touched:** `crates/mika-agent/src/tools/pr_merge_with_gate.rs`.

**Approximate diff size:** +40 lines (one helper function or inline block, a few error-path branches, logging). The helper could be named `write_auto_merge_pr_url_to_supervisor` and live in the same file as a free function or a `pub(crate)` impl method.

### Phase 2 — Unit / integration tests

1. **Unit test: `pr_merge_with_gate.rs` test module.**
   - Test name: `test_auto_merge_enabled_writes_pr_url_to_supervisor`. Build an in-memory DB. Seed a manual `self_dev` supervisor task. Seed a callback child of the supervisor with `trigger_type='callback'`. Construct a `ToolContext` with `callback_task_id = Some(child_id)`. Mock the `gh` CLI calls so `pr_view` returns a non-conflicting state, `view_checks` returns pending, and `run_gh_merge(auto_merge=true)` returns Ok. Invoke the tool. Assert the result is `MergeGateResult::AutoMergeEnabled`. Assert the supervisor's metadata now contains `$.claude_pilot.pr_url` matching the constructed URL.
   - Test name: `test_auto_merge_enabled_skips_metadata_when_no_callback_context`. Same setup but with `callback_task_id = None`. Assert the tool still returns `MergeGateResult::AutoMergeEnabled` and the supervisor's metadata is unchanged.
   - Test name: `test_auto_merge_enabled_skips_metadata_when_parent_not_self_dev`. Seed a supervisor with `source = Some("operator")` (not self_dev). Assert the tool returns `AutoMergeEnabled` and the supervisor's metadata is unchanged.
   - Test name: `test_auto_merge_enabled_skips_metadata_when_parent_not_manual`. Seed a parent with `trigger_type='callback'`. Assert skip.
   - Test name: `test_auto_merge_enabled_metadata_write_failure_does_not_fail_tool`. Force the `update_task_metadata` to fail (close the DB or mock failure). Assert the tool still returns `AutoMergeEnabled` (failure logged, not propagated).
   - Test name: `test_auto_merge_enabled_metadata_write_is_idempotent`. Pre-seed the supervisor with `metadata = '{"claude_pilot":{"pr_url":"https://..."}}'`. Invoke the tool. Assert no error, metadata still has the same pr_url (re-writing the same URL is fine), and other claude_pilot fields (if any) are preserved.

2. **Integration test: reaper interaction.**
   - In `crates/mika-agent/src/task_engine/engine.rs` reaper test module, add `test_reaper_skips_supervisor_after_auto_merge_enabled`. Seed the canonical reaper-bait state (supervisor `in_progress`, delivered child older than grace, no pr_url). Run `pr_merge_with_gate` (via direct call or test-fixture invocation that writes pr_url). Then call `reap_orphaned_parent_tasks`. Assert the supervisor is still `in_progress` (not reaped).

3. **Integration test: parent-completer interaction.**
   - In the same module, add `test_parent_completer_promotes_supervisor_after_auto_merge_enabled`. Same seeding. After `pr_merge_with_gate` writes pr_url, advance the clock past grace (or use a test-only short grace). Call `complete_parent_tasks_on_callback_success`. Assert the supervisor is `completed`.

4. **Regression: existing tests for pr_merge_with_gate, reaper, parent-completer all pass unchanged.** No existing test asserts the supervisor stays without pr_url after `AutoMergeEnabled`, so no regression breakage. Verify by running `cargo test -p mika-agent --tests`.

### Phase 3 — Verification and docs

1. **Manual verification on a clean autonomous dispatch.** Pick a small, low-risk open ticket (e.g., a docs-only fix). Dispatch through the autonomous loop. When QA passes and `pr_merge_with_gate` returns `auto_merge_enabled`, check the supervisor's metadata via:
   ```bash
   sqlite3 ~/.mika/data/mika.db "SELECT id, status, metadata FROM tasks WHERE id = '<supervisor_id>';"
   ```
   Confirm `$.claude_pilot.pr_url` is populated. Wait for one reaper tick cycle (60s) and confirm the supervisor stays `in_progress`. Wait for grace + one tick (≥660s) and confirm the parent-completer transitions the supervisor to `completed`.
2. **Compound doc.** Add `docs/solutions/best-practices/pr-merge-with-gate-supervisor-metadata-2026-05-20.md` documenting the structural pattern: "any tool that establishes a positive PR-existence signal MUST write `$.claude_pilot.pr_url` to the supervisor's metadata, so the reaper / parent-completer pair behaves correctly." Cross-link to mika#1211, mika#871, mika#1162.
3. **CLAUDE.md note.** In `crates/mika-agent/CLAUDE.md` § PR Merge Gate, add a one-line bullet: "On `auto_merge_enabled`, the tool writes `$.claude_pilot.pr_url` to the supervisor task (resolved via `ToolContext.callback_task_id → parent`) so the orphan reaper and parent-completer behave correctly. See `docs/solutions/best-practices/pr-merge-with-gate-supervisor-metadata-2026-05-20.md`."

## Verification plan

### Build / static checks

- `cargo build -p mika-agent` — succeeds.
- `cargo clippy -p mika-agent` — no new warnings.
- `cargo fmt --check` — clean.

### Unit / integration tests

- `cargo test -p mika-agent --lib tools::pr_merge_with_gate` — new tests in Phase 2 step 1 pass.
- `cargo test -p mika-agent --lib task_engine::engine` — Phase 2 steps 2–3 pass, existing reaper / completer tests unchanged.
- `cargo test -p mika-agent` — full crate test suite passes (~3463 tests).

### Behavioral verification

- Manual autonomous-loop dispatch (Phase 3 step 1) — supervisor metadata has pr_url, reaper skips, parent-completer completes.
- Negative path: `pr_merge_with_gate` called in conversation mode (e.g., mika-arch chat) — tool behavior unchanged, no metadata write attempted.

### Observability

- `info!` log entry `pr_merge_with_gate: wrote pr_url to supervisor metadata on auto_merge_enabled` visible in the server log (`$MIKA_SERVER_LOG_FILE`) on every `AutoMergeEnabled` path that has a callback context.
- Existing audit events (`task_engine_parent_completer`) continue to fire on the eventual transition.

## Risks and mitigations

- **R-risk-1: Writing pr_url makes the parent-completer fire optimistically before the PR actually merges.** The parent-completer will transition the supervisor `in_progress → completed` after 600s grace, even though the PR may still be queued. Mitigation: this matches the existing semantic of the `Merged` path (the parent-completer doesn't verify actual merge state there either). Operator forensics rely on PR state in GitHub, not supervisor status. Acceptable.
- **R-risk-2: PR closed without merge after auto-merge enabled leaves the supervisor `completed` (incorrect).** Same shape as R-risk-1. Out of scope for this ticket; a follow-up could add a "PR closed without merge" webhook handler that transitions the supervisor `completed → failed`. Acceptable to ship without this.
- **R-risk-3: Writing pr_url to the wrong task (a mis-resolved supervisor).** Mitigated by the resolution guards in Phase 1 step 1: only manual `self_dev` parents of the callback context are touched. Other shapes (milestone/project parents, non-self_dev sources, conversation context) are skipped.
- **R-risk-4: Idempotency violation if mika-dev's `update_task_status` writes a different pr_url shape later.** Unlikely — `update_task_status`'s metadata merge uses the same two-level shallow merge. If mika-dev writes the same pr_url, it's a no-op. If mika-dev writes a different URL (e.g., to a fork), the later write wins — which is the same semantic as today. No new risk.
- **R-risk-5: `gh` CLI rate limit or DB lock under high load when the metadata write fires.** The write is best-effort, fire-and-forget; failure is logged and does not propagate. Worst case the supervisor leaks the same as today (no regression). Acceptable.

## Acceptance criteria

- **AC1.** `pr_merge_with_gate` returns `MergeGateResult::AutoMergeEnabled` and, in a callback turn whose parent is a manual `self_dev` supervisor, writes `$.claude_pilot.pr_url = "https://github.com/senara-solutions/<repo>/pull/<n>"` to the supervisor's metadata. Verified via unit test in Phase 2 step 1.
- **AC2.** After `pr_merge_with_gate` writes pr_url, the reaper (`reap_orphaned_parent_tasks`) skips the supervisor on subsequent ticks. Verified via integration test in Phase 2 step 2.
- **AC3.** After `pr_merge_with_gate` writes pr_url, the parent-completer (`complete_parent_tasks_on_callback_success`) transitions the supervisor `in_progress → completed` once `child.updated_at` is older than `REAPER_GRACE_SECONDS`. Verified via integration test in Phase 2 step 3.
- **AC4.** `pr_merge_with_gate` called outside a callback context (conversation mode) is unchanged: same `MergeGateResult` return, no metadata write attempted, no error logged. Verified via unit test in Phase 2 step 1 (the "no callback context" variant).
- **AC5.** Metadata-write failure (DB error, etc.) is logged at `warn!` but does not change the tool's `MergeGateResult` return value. Verified via unit test in Phase 2 step 1 (the "metadata write failure" variant).
- **AC6.** Full `cargo test -p mika-agent` passes with no regressions.

## Open questions

1. **Should the same write fire on `MergeGateResult::Merged` and `::AlreadyMerged` paths?** Today's reaper / completer pair handles those paths via `try_extract_callback_metadata`'s parsing of dispatch-lib's `PR:` line. If that misses (same race conditions discussed in Problem Frame), the merged path leaks a supervisor symmetric to the auto-merge case. The scoped decision in D7 is to limit this fix to `AutoMergeEnabled`; a follow-up ticket can extend symmetrically. Architect feedback welcome on whether to broaden scope here vs. follow-up.
2. **Should the metadata write also include `auto_merge_pending: true` for operator-visible status discrimination?** Today there's no consumer for that flag, but operator forensics ("is this PR queued or actually merged?") could benefit. Architect feedback welcome on whether to add this scaffolding now or defer until a consumer materializes.

## Implementation order

1. Read `crates/mika-agent/src/tools/pr_merge_with_gate.rs` end-to-end. Locate the `AutoMergeEnabled` branch (`pr_merge_with_gate.rs:192-224`).
2. Read `crates/mika-agent/src/task_engine/dispatcher.rs:1224-1283` (`try_extract_callback_metadata`) as the reference pattern for the metadata write.
3. Implement the supervisor resolution + metadata write in the `AutoMergeEnabled` branch.
4. Add the unit tests in `pr_merge_with_gate.rs`'s test module.
5. Add the integration tests in `task_engine/engine.rs`'s test module.
6. Run `cargo test -p mika-agent`. Iterate on failures.
7. Run `cargo clippy -p mika-agent`. Address warnings.
8. Update `crates/mika-agent/CLAUDE.md` § PR Merge Gate with the one-line note.
9. Write the compound doc in `docs/solutions/best-practices/`.
10. Stage commit. Hand off to /mika pipeline for /ce:review and PR.
