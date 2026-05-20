# Plan: design(self-dev) HOLD re-entry semantics for M4 milestone loop — mika#1208

type: design
ticket: mika#1208
date: 2026-05-20
parent: mika#789 (introduces HOLD state without resolving re-entry)
related: mika#991 (callback_milestone_advance guard, PostCallbackAdvance backstop), mika#1124 (deferred-promotion wedge — adjacent class)
unblockers: mika#1207 (milestone-close-claim guard — landed in commit 498c536a; mika-arch unblocked for milestone-workflow reviews)
coupled-followup: mika#1218 (webhook_milestone_advance INTENT_GUARD — engine-layer structural parity with callback path)
base SHA: 498c536a18de83f69216aefc330d321f22277163

## Summary

Pick a transition rule for "M4 sees a child in HOLD" and close the resume gap that mika#789 left open. mika#789 added a HOLD state to M4 step 2.5 when `pr_merge_with_gate` returns `auto_merge_enabled`, but the prompt's claim that the loop will "re-enter M4 step 3 for this child" when the merge webhook fires is not backed by any code path or prompt instruction. The merge webhook handler (`self-dev-webhook-qa` Path A) marks the child `completed` and stops — it never advances the milestone.

**Recommendation:** **Option (a) — HOLD ends the turn; the webhook handler owns resume.** Make M4 step 2.5 a turn-boundary state (explicit `EndTurn` after setting the HOLD note), and extend `self-dev-webhook-qa` Path A with an explicit milestone-advance step that mirrors `self-dev-callback`'s pattern (find next pending child, dispatch — or transition the parent to `completed` if none remain). The companion engine-layer `webhook_milestone_advance` INTENT_GUARD is filed as **mika#1218** (coupled follow-up); this plan keeps the immediate fix prompt-only with an explicit `⚠ ENGINE GUARD PENDING mika#1218` warning in the prompt diff (per first-pass architect F2).

**Why not (b):** Option (b) (iterate-over-non-HOLD) would let M4 dispatch the next child while the prior child's PR is unmerged, which is exactly the invariant mika#789 step 2.5 was added to defend: *"This prevents dispatching the next ticket against code not yet on main."* Option (b) trades the verify-post-state guarantee for a "no-op iteration" that is hard to bound (HOLD-timeout becomes new control flow) and re-introduces the parallelism class that mika#727 exposed.

**Why not full "code over prompts" engine cascade now:** A clean engine-side cascade (a `MilestoneAdvance` SilentTrigger emitted whenever any milestone-child transitions to `completed`, regardless of trigger source) is architecturally cleaner and aligns with `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19`. The minimal first step toward that cascade — a `webhook_milestone_advance` INTENT_GUARD symmetric to `callback_milestone_advance` — is filed as mika#1218 and tightly coupled to this PR (mika#1218 AC3 removes the warning prose this PR adds). The fuller `MilestoneAdvance` SilentTrigger sits behind mika#1218 as an open architectural question; see §Follow-ups.

## Phase 0 — Code pins (verbatim slices at base SHA `498c536a`)

Per first-pass architect F1: verbatim slices at a named base SHA for all three edit sites + the callback mirror pattern this plan transplants, each with ~5 lines of surrounding context. Source pins are read-only; the implementer must compare these against `git show 498c536a:<path>` before editing to confirm no upstream drift.

### Pin A — `skills/bundled/self-dev/system_prompt.md` § Step M4 step 2.5 (current text — to be rewritten)

Lines 485–504:

```text
   - Close out child task

2.5. **Merge verification gate (verify-post-state):**

   After the QA webhook handler processes a `pass` verdict for this child's PR:

   - If `pr_merge_with_gate` returned `"merged"` or `"already_merged"`: **verify before advancing.** Call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and confirm `state == "MERGED"`. Only then proceed to step 3 with outcome `completed`. If state is not MERGED (race condition), treat as HOLD.
   - If `pr_merge_with_gate` returned `"auto_merge_enabled"`: the PR is NOT yet merged. This is a **HOLD state** — the child task stays `in_progress`. Do NOT advance to step 3. Do NOT dispatch the next child. Wait for the `pull_request.closed(merged: true)` webhook to arrive (handled by `self-dev-webhook-qa` → "Webhook Entry Point — PR Closed"). When the webhook arrives and the task transitions to `completed`, **verify before re-entering M4:** call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and treat only `state == "MERGED"` as merge success. Only then re-enter M4 step 3 for this child.
   - If `pr_merge_with_gate` returned `"blocked"` or `"gate_errored"`: the webhook handler already routed to the appropriate block/error path. M4 step 3 will see the child as `blocked`.

   **Literal verification command** (per committed decision — do NOT re-derive):
   ```
   run_gh(command=["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")
   ```
   Treat only `state == "MERGED"` as merge success. Any other state → HOLD.

   **Rule:** `auto_merge_enabled` is an intent signal, not a completion signal. The child stays in the serial execution slot until the merge webhook confirms actual merge AND `run_gh pr view` verifies `state == "MERGED"`. This prevents dispatching the next ticket against code not yet on main.

   **Incident:** mika#727 — KG milestone #14, PR #726 had auto-merge enabled but CI failed; next ticket #689 was dispatched against missing code.
```

**Pinpoint claim being modified:** the `"auto_merge_enabled"` bullet at line 492 plus the `"merged"` / `"already_merged"` bullet's tail clause at line 491 (`treat as HOLD`). The `"blocked"` / `"gate_errored"` bullet (line 493) and the `**Rule:**` / `**Incident:**` paragraphs (lines 501–503) are unchanged.

### Pin B — `skills/bundled/self-dev-webhook-qa/system_prompt.md` § Webhook Entry Point — PR Closed (current text — to receive new step 5.5)

Lines 154–172:

```text
### Webhook Entry Point — PR Closed (auto-merge completion)

**Path A — When `pull_request.closed` (merged: true) webhook arrives:**

> **CRITICAL: DO NOT end your turn without acting.**

When you receive a GitHub webhook event for `pull_request.closed`:

1. **Verify `merged: true`** in the event payload — if `merged` is false or absent, this PR was closed without merging. Ignore it (no action needed).
2. **Correlate:** match PR URL from event to task with `verdict_merge: auto` in metadata via `list_tasks(status: "in_progress")`.
3. **If no match found:** `send_message` to Vincent "Received merged webhook for {PR URL} but no matching task found. Manual check needed." Then stop.
4. **Pull main:** `git_ops({"operation":"fetch","repo_path":"<platform_dir>/<repo>/"})` then `git_ops({"operation":"merge","base":"origin/main","repo_path":"<platform_dir>/<repo>/"})`
5. **Complete task:** `update_task_status(status: "completed")`
6. **Notify:** `send_message`: "PR {repo}#{pr_number} auto-merged by GitHub. Task {label} complete. {PR URL}"

---

## Verdict Class: `recover_unpushed_work` (callback-originated, NOT webhook-originated)
```

**Insertion boundary:** new step 5.5 is inserted **between** existing step 5 and step 6 (between lines 166 and 167). The `---` separator at line 169 marks the end of Path A; insertion does not cross it. Path B (recover_unpushed_work) at line 171+ is untouched.

### Pin C — `skills/bundled/self-dev-callback/system_prompt.md` § Callback Entry Point (mirror pattern source)

Lines 1–13 (the precedent this plan transplants to the webhook):

```text
### Callback Entry Point (post background task)

**Engine contract (mika#991):** This turn is enforced by the `callback_milestone_advance` intent-precondition guard. For milestone/project-context callbacks, you MUST advance the queue or halt explicitly. The deliberation pattern ("Task X done, want me to proceed?") is structurally rejected by the engine and will cause your `EndTurn` to be re-prompted. If the first callback turn does not advance, the engine fires a `PostCallbackAdvance` second turn as a structural backstop; if that also fails, the engine auto-blocks the milestone.

When you receive a callback result from a completed background task (`run_claude_pilot` or `deploy_mika`):

> **CALLBACK TYPE DETECTION (MANDATORY — before any other processing):**
> Call `check_task(task_id)` on the callback's task. Read the `label` field to determine the callback type:
> - Label starts with `long_running:run_claude_pilot` → **claude-pilot callback**. Process using the claude-pilot handling below.
> - Label starts with `long_running:deploy_mika` → **deploy hook callback**. Skip metadata extraction (no session/cost/turns data). Check milestone context via `parent_task_id` as normal. On success, advance to the next child in M4 (step 4). On failure, pause milestone per step 3b.5.
> - Other labels → treat as claude-pilot callback (fallback).

> **CRITICAL: DO NOT end your turn after receiving a callback.** You MUST make at least one tool call before your turn ends. Generating a text summary without tool calls is a workflow failure. This rule is structurally enforced by the engine — callback turns with zero successful tool calls will be rejected and you will be re-prompted (#870).
```

**Transplant note:** the callback handler's "advance OR halt" obligation comes from BOTH (a) the inline `callback_milestone_advance` engine guard (mika#991) AND (b) the prompt prose. The webhook handler under this plan acquires only (b) at first; (a) lands via mika#1218. This is the load-bearing asymmetry F2 names — the prompt rewrite carries the `⚠ ENGINE GUARD PENDING mika#1218` warning explicitly.

### Pin D — Guard (0) `unauthorized_webhook_dispatch` (the surface F3 requires analysis of)

`crates/mika-agent/src/webhook_dispatch.rs` lines 32–49:

```rust
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}
```

Companion test (lines 109–120) — the Row E case:

```rust
// Row E — PR events → false (qa skill territory)
assert!(
    !is_unauthorized_webhook_dispatch(
        "[GitHub] PR opened: senara-solutions/mika#1000 — title (branch: foo)"
    ),
    "Row E: PR opened must be allowed (qa skill territory)"
);
assert!(
    !is_unauthorized_webhook_dispatch(
        "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"
    ),
    "Row E: PR closed must be allowed (qa skill territory)"
);
```

Gateway-side producer at `crates/mika-gateway/src/github.rs` line 384 (the format string that determines the originating-message prefix the predicate inspects):

```text
"[GitHub] PR {action}: {repo_name}#{number} — {title} (branch: {branch})\n{url}"
```

For `pull_request.closed(merged:true)`, `{action}` is `closed`, so the gateway emits `[GitHub] PR closed: ...`, which matches the `msg.starts_with("[GitHub] PR ")` allowlist branch of `is_unauthorized_webhook_dispatch` (line 40 of `webhook_dispatch.rs`).

### Phase 0 conclusion (F3-grounded)

**Guard (0) does not reject `run_claude_pilot` calls from `pull_request.closed` webhook turns.** The `[GitHub] PR ` prefix is on the qa-skill-territory allowlist (positive allowlist per mika#1102, defense-in-depth pre-hoc check per mika#933 § Dispatch-readiness guard (#525)). Both `is_unauthorized_webhook_dispatch` (Pin D) and the post-hoc `webhook_no_unauthorized_dispatch` INTENT_GUARD (`crates/mika-agent/CLAUDE.md` § Post-Conditions step 6 entry b) consume the same shared predicate post-mika#1102 — there is no second guard layer that could reject. The `block[ac]` and `block[ci]` paths in `self-dev-webhook-qa` already exercise this allowance: the structural `verdict_handler` (`crates/mika-agent/CLAUDE.md` § Structural Verdict Handler) dispatches `claude-pilot` for AC-fix / CI-fix from PR-review webhook turns. The Path A milestone-advance step proposed in Phase 2 is structurally equivalent — same prefix family, same dispatch tool, same guard topology.

mika#1205's failure class was about deferred-dispatch wrappers, not guard (0) on PR-closed turns specifically; the F3 risk family is real for some webhook prefixes (Row C, D, H — issue actions, comments, unknown) but **not for the prefix this plan dispatches against** (`[GitHub] PR closed:`, Row E).

## Scope

- **In scope:**
  - `skills/bundled/self-dev/system_prompt.md` § Step M4 step 2.5 — rewrite per Phase 1 below.
  - `skills/bundled/self-dev-webhook-qa/system_prompt.md` § Webhook Entry Point — PR Closed (Path A) — insert step 5.5 per Phase 2.
  - **Fragility-acknowledgment warning** in both prompt diffs citing mika#1218 (per first-pass F2).
  - **Filed companion ticket mika#1218** for the engine-layer `webhook_milestone_advance` INTENT_GUARD.
- **Out of scope:**
  - The `webhook_milestone_advance` engine guard implementation itself (lands in mika#1218; mika#1218 AC3 removes the warning prose this plan adds).
  - A unified `MilestoneAdvance` SilentTrigger across callback/webhook/manual paths. Open question on mika#1218 (see §Follow-ups).
  - QA verdict `hold[*]` terminology (distinct surface, handled elsewhere — one-sentence disambiguation note in Phase 1 only).
  - Project workflow P4 — Project Step P4 explicitly says "Same as Milestone Step M4" (`self-dev/system_prompt.md:660`); the M4 rewrite carries over by reference, no textual duplication.

## Phase 1 — M4 step 2.5 rewrite (turn-boundary HOLD)

### Goal

Make HOLD an explicit turn boundary. The LLM's contract: "On HOLD, persist the HOLD note via `update_task_status` and end the turn. Do NOT iterate. The next dispatch is the webhook handler's job."

### Edit site

Pin A (above) — `skills/bundled/self-dev/system_prompt.md` lines 487–501.

### Proposed rewrite (text to replace the `auto_merge_enabled` bullet at line 492 and to append the idempotency rule after line 501's `**Rule:**` paragraph)

The `auto_merge_enabled` branch (line 492) becomes:

> - If `pr_merge_with_gate` returned `"auto_merge_enabled"`: the PR is NOT yet merged. This is a **HOLD state**. Persist the HOLD via `update_task_status(task_id=<child_task_id>, status="in_progress", note="HOLD: auto-merge enabled, awaiting pull_request.closed webhook (PR #<num>)")`. **End the turn immediately.** Do NOT loop. Do NOT dispatch the next child. Do NOT call `run_claude_pilot` again. The next M4 step for this milestone runs only when the `pull_request.closed(merged: true)` webhook arrives (handled by `self-dev-webhook-qa` → Webhook Entry Point — PR Closed, which is responsible for milestone advancement after marking the HOLD child `completed`).
>
> *(M4 HOLD ≠ QA verdict `hold[*]`. The latter is a verdict class for blocked-but-fixable PRs handled in `self-dev-webhook-qa` § Verdict class `hold[*]`. Same word, different machinery.)*
>
> > ⚠ **ENGINE GUARD PENDING mika#1218** — the "advance OR halt" obligation in the webhook handler (Phase 2 below) is enforced by prompt prose only until mika#1218 lands a `webhook_milestone_advance` INTENT_GUARD. This is the same against-gradient-behavior class as `callback_milestone_advance` (mika#991): the LLM's trained default is "acknowledge and close the turn" rather than "advance the queue." See `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` for the doctrine. mika#1218's AC3 removes this warning when the engine guard lands.

A new paragraph is appended to step 2.5 (after the existing `**Rule:**` and `**Incident:**` paragraphs, lines 501–503):

> **Idempotent re-entry:** if a callback turn or `PostCallbackAdvance` backstop re-enters M4 and finds the current child still in HOLD (status `in_progress`, note begins with `HOLD: auto-merge enabled`), this is a no-op turn. Do NOT re-dispatch. Do NOT loop. End the turn after the no-op tool call (`check_task` to observe HOLD state). If a `PostCallbackAdvance` (mika#991) backstop fires while the child is still HOLD, surface to the operator via `send_message` and call `update_task_status(parent_milestone_task_id, status='blocked', note='HOLD child not yet merged after PostCallbackAdvance — auto-merge may be stuck; operator review')`. (Engine improvement to recognize the HOLD note and skip the backstop is folded into mika#1218.)

## Phase 2 — Webhook Path A milestone-advance

### Goal

Make the merge webhook responsible for milestone advancement — mirror the callback handler's "advance or halt" contract (Pin C).

### Edit site

Pin B (above) — `skills/bundled/self-dev-webhook-qa/system_prompt.md` lines 154–172.

### Proposed insertion (new step 5.5, between existing step 5 and step 6)

```text
5.5. **Milestone/project advance gate (parity with self-dev-callback § Permitted post-callback actions item 2):**

   ⚠ **ENGINE GUARD PENDING mika#1218** — this gate is prompt-prose-only until mika#1218 lands a `webhook_milestone_advance` INTENT_GUARD symmetric to `callback_milestone_advance` (mika#991). Until then, the contract below is enforced only by this prompt. mika#1218's AC3 removes this warning when the engine guard lands.

   Call `check_task(parent_task_id)`. If `parent_task_id` is null OR the parent's `type` is neither `"milestone"` nor `"project"`, proceed to step 6 (notify and stop — non-milestone task, no advance needed).

   Otherwise, the completed child is a milestone/project child. You MUST advance per M4 step 3 (or P4 step 3 for projects). Choose exactly one path:

   **5.5.a Verify PR actually merged.** Call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")`. If `state != "MERGED"`, the webhook is racing (or fired for a non-merge close). Re-set the child to HOLD via `update_task_status(child_task_id, status="in_progress", note="HOLD: webhook arrived but PR state != MERGED; awaiting confirmation")`, notify Vincent via `send_message`, and end the turn. Do NOT advance.

   **5.5.b Deploy hook check** (mirrors M4 step 3b — `self-dev/system_prompt.md:512-536`). Read the child task's `metadata.labels`. If `needs-build` or `needs-deploy` is present:
   - Notify Vincent: "Deploy hook triggered for <repo>#<issue> via auto-merge webhook (label: <label>). Running build+deploy before next ticket."
   - Call `deploy_mika({"task_id": "<milestone_wi>"})`.
   - End the turn. The deploy callback drives the next iteration via `self-dev-callback` (which inherits the milestone-context advance contract).

   If no deploy-hook label is present, continue to 5.5.c.

   **5.5.c Find and dispatch the next pending child.** Call `list_tasks(parent_task_id=<milestone_wi>)`. Filter to `type="issue"` children with status `pending`. Order by `created_at` ascending (the topo-sorted dispatch order set at M3 time).

   - **If a next pending child exists:** Transition it to `in_progress` via `update_task_status(<next_child>, status="in_progress")`, then call `run_claude_pilot(...)` with the next child's `task_id` per M4 step 1 + step 2. The `run_claude_pilot` call itself ends the turn (claude-pilot fires asynchronously; the next M4 iteration lands on the resulting callback turn). Guard (0) `unauthorized_webhook_dispatch` does NOT reject this call — `[GitHub] PR closed:` is on the qa-territory allowlist per `crates/mika-agent/src/webhook_dispatch.rs:40` (mika#933) and Row E test at the same file's `test_is_unauthorized_webhook_dispatch_predicate`.

   - **If no pending children remain (this was the last):** This is the milestone-completion path. M5 lives in `self-dev/system_prompt.md` and is not directly reachable from the webhook handler today (see §Risks §R1). Update the milestone parent and surface to the operator: `update_task_status(parent_task_id=<milestone_wi>, status="in_progress", note="Auto-merge of <repo> issue#<issue> completed via webhook — operator-resume needed to drive M5 close-out")`, then `send_message`: "Milestone <repo> milestone#<n> last child auto-merged via webhook. Reply 'continue' to run M5 close-out."

   **Forbidden actions in this turn:** Acknowledge-and-close ("PR #<num> merged. Task complete.") without one of 5.5.a, 5.5.b, or 5.5.c above. The engine `webhook_milestone_advance` guard (mika#1218) will reject this structurally when it lands; until then, this rule is prompt-only.
```

(Existing step 6 — the `send_message` notification — runs unchanged for the non-milestone case in 5.5 step 1, and is otherwise subsumed by 5.5.a / 5.5.b / 5.5.c which each contain their own `send_message` callouts.)

### Engine-guard symmetry note (per first-pass F2)

The deferral choice is **Position 3 from the first-pass brief** — prompt-only this PR + coupled-follow-up filed as mika#1218. The first-pass architect's framing pushed against keeping this as a "prompt-only forever" risk; the mitigations applied per their F2 (b):

1. **Follow-up ticket filed:** mika#1218 (linked above), with stated coupling — mika#1218's AC3 removes the warning prose this plan adds.
2. **Fragility acknowledgment in plan:** see this section + §Risks §R2.
3. **`⚠ ENGINE GUARD PENDING mika#1218` warning in prompt diff:** appears in BOTH the M4 step 2.5 rewrite (Phase 1) AND the webhook step 5.5 insertion (Phase 2), at the top of each affected block, where the LLM will see it on every turn until mika#1218 lands.

## Phase 3 — Test coverage

The existing eval harness at `crates/mika-agent/tests/eval/test_callback_milestone_advance.rs` (referenced in `crates/mika-agent/CLAUDE.md` § Silent Mode Agent Loop) owns the milestone-advance invariants. The webhook-cascade path needs:

1. **Webhook PR-closed advances next child** — a milestone child is marked `in_progress` with the canonical HOLD note (`"HOLD: auto-merge enabled, ..."`) and a `pull_request.closed(merged:true)` webhook fires. The test verifies: (a) the HOLD child transitions to `completed`, (b) `run_gh pr view` is called with `state,mergedAt` and the mock returns `MERGED`, (c) `list_tasks(parent_task_id=<milestone>)` finds the next pending child, (d) `run_claude_pilot` is called for that child within one turn.
2. **Webhook PR-closed → operator notification on last child** — the HOLD child is the last pending child. The test verifies (a) child transitions to `completed`, (b) `update_task_status` is called on the milestone parent with the "operator-resume needed to drive M5 close-out" note, (c) `send_message` is called with the `continue` prompt, (d) `run_claude_pilot` is NOT called.
3. **Webhook PR-closed → deploy-hook path** — child labels include `needs-deploy`. The test verifies `deploy_mika` is called with the milestone parent's task_id and `run_claude_pilot` is NOT called.
4. **Idempotent HOLD re-entry on `PostCallbackAdvance`** — a `PostCallbackAdvance` SilentTrigger fires while the child is still HOLD (webhook hasn't arrived yet). The test verifies: (a) no `run_claude_pilot` call, (b) no status change, (c) the LLM surfaces the "HOLD child not yet merged" notification + blocks the parent milestone per Phase 1's idempotent rule.
5. **PR state race — webhook arrives but `state != MERGED`** — the test verifies the child is re-set to HOLD with the "webhook arrived but PR state != MERGED" note and the milestone is NOT advanced.

**Placement decision:** extend `test_callback_milestone_advance.rs`. The invariant under test is "milestone advance on completion" — same across event sources; the trigger type is a parameter. New cohort sub-module within the existing file rather than a sibling file. (This resolves my first-pass uncertainty (4) in favor of the cohort-by-invariant cut; the architect can override on second pass if "cohort by event source" is preferred.)

## Phase 4 — Documentation

1. Add `docs/solutions/workflow-issues/m4-hold-re-entry-semantics-2026-05-20.md` capturing the design call: why option (a), why not (b), why mika#1218 is a coupled-follow-up rather than in-scope-here. Frontmatter: `module: self-dev, agent-core`; `tags: [milestone-cascade, webhook, hold-state, engine-guards-vs-prompts]`; `problem_type: workflow_issue`; `applies_when:` list including "introducing a new completion path that advances a queue but cannot trivially reach the source handler's queue-management logic." Cross-link from `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` as the next data point in the gradient.
2. Update `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b to note that the webhook companion guard is filed as mika#1218 and that until it lands the prompt-only contract carries the obligation.

## Risks

1. **R1 — M5 close-out from webhook turn (asymmetry).** When the last pending child auto-merges via webhook, the cleanest answer is for the webhook turn to invoke M5 step 1–6 (stats, build+deploy, close milestone, update memory, notify). M5 currently lives in `self-dev/system_prompt.md:540-592` and is not reachable from the webhook handler. Phase 2 step 5.5.c defers this to operator `continue`. **Decision options:**
   - **(R1.a)** Accept the asymmetry; operator drives M5 on webhook-last-child. Cheapest; leaks the autonomous-loop guarantee.
   - **(R1.b)** Duplicate M5 logic in the webhook handler. Costly maintenance; M5 is non-trivial; introduces drift class.
   - **(R1.c)** Add an engine-side `MilestoneCompletionCascade` SilentTrigger that fires `self-dev`'s M5 from any context. Bigger change; folds into mika#1218 as an open architectural question.
   - **My lean:** (R1.a) for this ticket. mika#1218 is the right home for (R1.c) once the smaller `webhook_milestone_advance` guard ships and the symmetry is established at the per-event-source layer.
2. **R2 — Prompt-only `webhook_milestone_advance` is permanent if mika#1218 is deprioritized (first-pass F2).** Mitigations: (a) the warning prose appears in BOTH prompt diffs (M4 step 2.5 AND webhook step 5.5), so the LLM sees it on every relevant turn until mika#1218 lands; (b) the warning explicitly cites mika#1218's AC3 (warning removal) — the warning is self-deleting on engine-guard landing; (c) this plan §Phase 4 step 2 updates `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b to name the gap. **Residual risk:** if mika#1218 sits in the backlog for >1 milestone, the autonomous-loop's milestone-cascade behavior on webhook completion is one LLM-drift away from a stall, and the failure mode looks identical to the pre-#789 mika#727 drift. Operator visibility on the warning prose + a milestone-completed-via-webhook count metric (could land in mika#1218 as AC5) is the operational mitigation.
3. **R3 — `PostCallbackAdvance` race against HOLD.** Phase 1's idempotent rule routes to "notify + block milestone." This is correct (the engine is asking "why did you not advance?" and the honest answer is "because the child is still HOLD"), but produces operator-notification spikes on slow merges. Engine improvement to recognize the HOLD note and skip the backstop is folded into mika#1218.
4. **R4 — `update_task_status(status: "in_progress")` on a child already `in_progress`.** Phase 1's HOLD persistence calls `update_task_status` with the same status the task already has. Per `crates/mika-agent/CLAUDE.md` § Status transition state machine, `in_progress -> in_progress` is not in the allowed transition set; need to verify the tool accepts same-status updates with metadata-only writes (the note: field). If it does not, the persistence step must use a different metadata-write mechanism (e.g., `check_task` + metadata-only update path). **Open for second-pass:** if the architect knows the answer, please confirm; otherwise this is a Phase 1 implementation discovery cost.

## Open questions for the architect (second-pass)

1. **R4 same-status update.** Does `update_task_status` accept same-status calls when only `note` changes? If not, what's the canonical metadata-write tool? (Looking at the `tasks.update_task_status` tool semantics — the schema doc says metadata can still be written on terminal states; less clear on same-status active-state updates.)
2. **Test cohort placement.** Confirming the "extend `test_callback_milestone_advance.rs`" choice over a new sibling file. (This resolves first-pass uncertainty (4) but ratifying or rejecting now saves a roundtrip.)
3. **`PostCallbackAdvance` race response shape.** Phase 1's idempotent rule transitions the milestone to `blocked` when a backstop fires against a HOLD child. Alternative: silent no-op (the LLM does nothing — the engine accepts). My lean is the louder "block + notify" path because a backstop firing while merge takes too long IS an anomaly the operator should see; but if the architect reads this as too-loud (creates false-positive notifications on healthy slow merges), the silent-no-op shape is acceptable.
4. **M5 close-out (R1) — final position.** Confirming (R1.a) for this ticket with (R1.c) inheriting into mika#1218. If (R1.c) needs to land here (the asymmetry is unacceptable for autonomous loop), Phase 2 step 5.5.c last-child branch must duplicate M5 inline, with the maintenance-drift caveat.

## Follow-ups (ticket-able after merge)

- **mika#1218** — already filed. `webhook_milestone_advance` INTENT_GUARD + tests + the `MilestoneCompletionCascade` SilentTrigger open question (R1.c). AC3 of mika#1218 removes both `⚠ ENGINE GUARD PENDING mika#1218` warnings added by this plan.
- **HOLD note canonicalization (engine-side recognition):** engine learns to parse the `"HOLD: auto-merge enabled"` note format and suppresses `PostCallbackAdvance` backstop turns while HOLD is active. Folded into mika#1218 scope per R3.
- **M4 HOLD timeout / staleness:** if HOLD persists more than N hours (default ~6h, configurable), surface to operator. Not in this ticket because the timeout dimension is option (b)-flavored. File separately if operational signals show a need.
- **`gh_read` parity for webhook handler:** the webhook handler currently uses `run_gh` for the PR state check; the architect agent uses `gh_read`. Out of scope; only relevant if the webhook-handler ever moves to the architect's read-only pattern.

## Acceptance criteria (for the implementation PR this plan will drive)

- AC1: `skills/bundled/self-dev/system_prompt.md` § M4 step 2.5 — `auto_merge_enabled` branch rewritten per Phase 1, including "End the turn immediately" + idempotent re-entry rule + HOLD-vs-`hold[*]` disambiguation + `⚠ ENGINE GUARD PENDING mika#1218` warning.
- AC2: `skills/bundled/self-dev-webhook-qa/system_prompt.md` § Path A — new step 5.5 inserted per Phase 2 (between current lines 166 and 167), including 5.5.a verification, 5.5.b deploy hook, 5.5.c next-child dispatch with last-child operator fallback, the `⚠ ENGINE GUARD PENDING mika#1218` warning, and the guard-(0) allowance citation.
- AC3: Eval tests added per Phase 3 — five scenarios, extending `tests/eval/test_callback_milestone_advance.rs` as a new cohort.
- AC4: `docs/solutions/workflow-issues/m4-hold-re-entry-semantics-2026-05-20.md` captures the design decision per Phase 4 step 1.
- AC5: `crates/mika-agent/CLAUDE.md` § Post-Conditions step 6b updated per Phase 4 step 2.
- AC6: Cross-repo grep confirms no other path claims "re-enter M4 step 3" without an actual mechanism (verify it doesn't recur in `self-dev-callback`, `self-dev-iterate`, or `self-dev-webhook-ci`).

## Out of scope (explicitly)

- Engine code changes — mika#1218 is the home; this PR is prompt-only with `⚠ ENGINE GUARD PENDING mika#1218` warning.
- Provider-specific prompt variants (self-dev has none; per `feedback_no_provider_prompts`).
- QA verdict `hold[*]` semantics (distinct surface).
- Project workflow P4 — inherits M4 by reference at `self-dev/system_prompt.md:660`; no textual duplication.
- M5 close-out from webhook context (R1.c) — folded into mika#1218 unless second-pass elevates it here.
