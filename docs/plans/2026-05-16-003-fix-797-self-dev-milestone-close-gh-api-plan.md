---
ticket: mika#797
title: "fix(self-dev): milestone-close workflow must call gh API, not just update local task state"
type: fix
status: groomed-pending-architect-pass1
author: orchestrator-Claude (mika-1)
date: 2026-05-16
revision: 2 — rewritten after pass-1 ESCALATE; operator chose option (a): block on mika#788, no override
---

# Plan: mika#797 — Milestone-close workflow must close on GitHub, not just locally

## 0. Phase 0 Pin (base + verbatim quotes of load-bearing sites)

**Base commit:** `447143c89f3377bbe3542c051c35a7c378e0a06a` (origin/main as of 2026-05-16).

**Site 1 — `crates/mika-agent/src/skills/builtin_handlers.rs:1619-1630`** (GH_ALLOWED_SUBCOMMANDS as it stands today; mika#788 will mutate this constant — adding `"api"`, removing `"milestone"` and `"project"` — before this ticket lands):
```rust
const GH_ALLOWED_SUBCOMMANDS: &[&str] = &[
    "pr",
    "issue",
    "run",
    "workflow",
    "release",
    "repo",
    "search",
    "label",
    "milestone",
    "project",
];
```

**Site 2 — `crates/mika-agent/src/skills/builtin_handlers.rs:1770-1782`** (allowlist enforcement; unchanged by #788):
```rust
fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, ToolOutput> {
    let args = parse_command_array(input)?;
    let subcommand = &args[0];
    if !GH_ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ToolOutput::error(format!(
            "gh subcommand '{subcommand}' is not allowed. \
             Permitted: {}.",
            GH_ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }
```

**Site 3 — `skills/bundled/self-dev/system_prompt.md:266`** (the prompt's enumeration of permitted `run_gh` subcommands; mika#788 will rewrite this line — its scope explicitly lists "Update the self-dev prompt line 285 (the enumeration of permitted run_gh subcommands)"):
> `gh api` is not allowed; permitted: `pr, issue, run, workflow, release, repo, search, label, milestone, project`.

**Site 4 — `skills/bundled/self-dev/system_prompt.md:515-528`** (Step M5 — the insertion target for this ticket):
```markdown
### Step M5 — Milestone completion

When all children processed:
1. Gather stats from child tasks via `list_tasks` filtered by `parent_task_id=<milestone_wi>`. Count how many children reached `completed` status.
2. **Build + deploy (gated on >=1 completed child):** If at least one child completed successfully, trigger a build (`build_mika` if available, or `run_shell` with `cargo build --release --features telemetry`) then deploy (`deploy_mika` with `task_id=<milestone_wi>`). [...]
3. Transition parent: `update_task_status(task_id=<milestone_wi>, status="completed")`
4. **Record to memory:** `store_fact(category="event", description="Milestone <repo> milestone#<n> completed. [...]")`
5. Notify Vincent with summary: [...]
```

**Site 5 — `crates/mika-agent/src/agent.rs:1216-1274`** (completion-claim guard `detect_completion_claim`; this plan extends with a sibling guard):
```rust
if !skip_remaining_guards
    && matches!(response.stop_reason, LlmStopReason::EndTurn)
    && !completion_claim_retry_done
    && let Some(keyword) = detect_completion_claim(&text)
{
    if tools.get("update_task_status").is_some()
        && !tools_called.contains("update_task_status")
    {
        // [...] reject and re-prompt once with active-tasks list
```

`detect_completion_claim` regex (`agent.rs:4579-4582`): `(?i)\b(merged|deployed|completed?|shipped)\b`. Mentions `merged`/`deployed`/`complete`/`completed`/`shipped` — does NOT include `closed`.

## 1. Problem restatement

Milestone#17 (2026-04-24 → 2026-04-25) closed in mika-dev's local DB but never on GitHub. Agent reported success while `gh api /repos/.../milestones/17 --jq .state` returned `"open"`. Vincent closed manually. The class is `feedback_verify_before_claiming` extended to GitHub-API state changes.

## 2. Upstream dependencies (sibling-ticket state, verified 2026-05-16)

- **mika#788** (OPEN, milestone "Self-dev / dev-loop reliability v2") — ships the allowlist change. Committed decisions (verbatim from issue body): *"Add `api` to `GH_ALLOWED_SUBCOMMANDS`"*, *"No dedicated `close_milestone` / `update_project_item` builtin tools. Rejected as YAGNI — one use case, and `gh api` covers it."*, *"Update the self-dev prompt line 285 (the enumeration of permitted run_gh subcommands) to reflect the real allowlist."* Body also names: *"Blocks: the self-dev verify-post-state ticket (sibling)"* — that sibling IS mika#797. **This ticket has a hard dependency on mika#788.**
- **mika#793** (OPEN, same milestone) — `pr_merge_with_gate` caller audit. Same self-dev prompt surface but different section (Rule 6, not Step M5). No expected line-number collision; sequencing is parallel-safe but if #793 lands first it edits the prompt around lines 268-280 (Rule 6 region) which does not overlap Step M5 (lines 515+).

**Sequencing rule for this ticket:** mika#788 MUST land before mika#797's PR opens. mika#793 may land in either order. The plan assumes the post-#788 engine + prompt state; pre-#788 deployment of this PR would dispatch `run_gh(["api", ...])` calls that the engine would reject at validation.

Action item (operator, P5 step 18): apply a GitHub-native `blockedBy: senara-solutions/mika#788` edge on issue #797 so the engine-side `validate_dispatch_readiness()` blockedBy check (#713) refuses to dispatch mika#797 implementation until #788 lands. Tooling: GraphQL mutation `addIssueDependency` via `gh api graphql` once #788's allowlist change lands (any earlier and the mutation itself can't be dispatched by mika-dev for the same allowlist reason — a chicken-and-egg the operator routes around manually).

## 3. Solution shape (option a: prompt-only + structural guard)

Three parts, all in one PR (no Rust tool code; just prompt, eval, and a 30-line guard extension).

### Part B — Self-dev prompt M5 update

`skills/bundled/self-dev/system_prompt.md`, Step M5 (Phase 0 Pin site 4). Insert a new step **3** between current 2 (build+deploy) and current 3 (`update_task_status(completed)`):

```markdown
3. **Close the GitHub milestone (REQUIRED before marking the parent task complete):**

   3a. Issue the close PATCH:
       run_gh({
         "command": ["api", "-X", "PATCH",
                     "/repos/senara-solutions/<repo>/milestones/<n>",
                     "-f", "state=closed"]
       })

   3b. Read back the state:
       run_gh({
         "command": ["api",
                     "/repos/senara-solutions/<repo>/milestones/<n>",
                     "--jq", ".state"]
       })

   3c. Branch on the readback:
   - Output is exactly `"closed"` (with quotes — `--jq .state` emits JSON): proceed to step 4.
   - Output is `"open"` or anything else: STOP. Do NOT call `update_task_status(completed)`.
     Notify Vincent: "Milestone <repo> milestone#<n> close PATCH returned 2xx but
     readback shows state=<value>. GitHub-side divergence; not marking local task complete."
     `update_task_status(task_id=<milestone_wi>, status="blocked",
     note="GitHub milestone close readback mismatch — got state=<value>")`.
   - 3a returns a non-2xx error: STOP. Notify Vincent with the gh error. Mark task `blocked`
     with the error in the note. Do NOT claim success.

   This step is load-bearing for the verify-before-claiming discipline. Engine-side guard
   (mika#797 part D) will reject EndTurn responses claiming "milestone closed" that did not
   invoke step 3a.
```

Renumber subsequent steps (current 3→4, 4→5, 5→6). Update the user-facing notification template (new step 6) to include the line `Milestone closed on GitHub: ✓` so the operator can distinguish a verified close from a claimed-but-unverified one.

**Project Workflow note:** Step P5 currently says "Same as Milestone Step M5." Add an inline NOTE inside P5 explicitly: *"The milestone close step (M5 step 3) is REST-specific to `/milestones/<n>`. GitHub Projects v2 closes via GraphQL `closeProjectV2` mutation and is OUT OF SCOPE for this ticket — see mika#TBD."* (Sibling ticket to be filed during /ce:work or as part of this PR's "Followups" section.)

### Part C — Grounding regression eval scenario

`crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs` (new file), registered in `mod.rs`.

**Scenario name:** `milestone_close_verify_before_claim`

**Setup:** Synthetic M5 turn shape. All children completed. Build+deploy already simulated as done (prior turn). Agent is presented with the M5 prompt continuation. MockLlmProvider configured to mock the LLM's response trajectory.

**Two sub-scenarios:**

**C1 — happy path (post-fix):**
- LLM response calls `run_gh(["api", "-X", "PATCH", "/repos/senara-solutions/mika/milestones/17", "-f", "state=closed"])`.
- Mock tool returns 2xx response.
- LLM response then calls `run_gh(["api", "/repos/senara-solutions/mika/milestones/17", "--jq", ".state"])`.
- Mock tool returns `"closed"`.
- LLM response calls `update_task_status(task_id="...", status="completed")`.
- LLM emits final text with "Milestone closed on GitHub: ✓".
- **Assertions:** `assert_any_tool_called_from(["run_gh"])` × 2 invocations with the expected argv shapes; response contains "Milestone closed on GitHub" only after the readback returned `"closed"`.

**C2 — pre-fix regression (frozen fixture):**
- Replays the milestone#17 trajectory: LLM emits "Milestone#17 closed, tasks reconciled, memory updated" with zero `run_gh` calls in the turn.
- **Assertions:** `detect_milestone_close_claim_without_patch()` (Part D guard) fires; EndTurn is rejected; correction message names the missing PATCH call.

Both scenarios use the `frozen-regression-fixture` pattern: `fixtures/milestone_close_pre_fix.json` captures the milestone#17 response shape; the regression-reproduction test proves Part D's guard catches it.

**Tag vocabulary additions:** `grounding:verify-before-claim-milestone` (sub-tag of the existing `verification-before-claim`).

### Part D — Structural guard: `detect_milestone_close_claim_without_patch`

New sibling guard alongside `detect_completion_claim` at `crates/mika-agent/src/agent.rs`. Same firing site as the completion-claim guard (post-condition #4 in the EndTurn chain, around `agent.rs:1216`).

**Trigger condition:**
1. `LlmStopReason::EndTurn`
2. Assistant text matches: `(?i)\bmilestone\b.{0,80}\b(closed|close)\b` (case-insensitive; "milestone" followed within 80 chars by "closed" or "close").
3. NO `run_gh` invocation in the turn whose argv satisfies BOTH: (a) contains the literal `"api"` at index 0, AND (b) contains the literal `"PATCH"` adjacent to `"-X"`, AND (c) any argv element matches the regex `^/repos/[^/]+/[^/]+/milestones/\d+$`.

**On trigger:** Reject EndTurn once (single retry, tracked via standalone `milestone_close_claim_retry_done` flag, same shape as `completion_claim_retry_done`). Correction message:

> Your response was rejected because you claimed a GitHub milestone was closed (matched: "<keyword>") but did not invoke `run_gh` with the close PATCH. Closing a milestone locally is not the same as closing it on GitHub — the previous incident (milestone#17, 2026-04-24) left local state and GitHub state divergent for hours.
> Call `run_gh({"command": ["api", "-X", "PATCH", "/repos/senara-solutions/<repo>/milestones/<n>", "-f", "state=closed"]})` AND verify via readback before claiming the milestone is closed, OR retract the claim if the close was not actually performed.

**Position in the post-condition chain:** insert AFTER the existing completion-claim guard (#4) and BEFORE the fabricated-action-claim guard (#5). New guard becomes #4b. Single retry per turn; standalone flag.

**Interaction with `detect_completion_claim` (F1 from pass-1 v2 architect review).** Both guards can match the same assistant text — e.g., "Milestone#17 completed and closed" contains both `completed` (existing guard's regex) and `milestone … closed` (new guard's regex). Interaction semantics:

- **One guard per turn, serial evaluation.** The post-condition chain (`agent.rs:1216`+) evaluates guards in numbered order and `continue`s on the first rejection. Existing completion-claim guard (#4) sits before the new milestone-close guard (#4b). If both regexes match, the completion-claim guard fires first and triggers a single corrective re-prompt; the new guard does NOT also fire on the same response. **No double-rejection within one turn.**
- **Independent retry flags, multi-turn behavior.** `completion_claim_retry_done` and `milestone_close_claim_retry_done` are separate `bool` flags. After the completion-claim retry lands and the agent's corrected response still violates the milestone-close discipline (e.g., adds `update_task_status` to satisfy #4 but still omits the PATCH call), the new guard can fire on the *next* turn — independent retry budget per failure class.
- **Order matters and is intentional.** Completion-claim is the broader catch (covers merge/deploy/ship as well as complete). Milestone-close is the specialized catch. Putting the broader guard first means an agent claiming "completed AND closed" without `update_task_status` gets corrected on the missing task transition first; if the agent then claims closure but did call `update_task_status`, the specialized guard catches the missing PATCH on a subsequent EndTurn.

**Why structural and not eval-only:** The eval scenario locks the failure class against regression in the eval harness, but the guard fires *in production* on real autonomous-loop turns. This is the structural application of `feedback_prompt_enforcement_fragile` that mika#788's "no dedicated tool" decision relies on — the prompt-level discipline of "verify before claiming" gets engine-side teeth.

**False-positive surface:** Text like "we should close the milestone tomorrow" or "the milestone is now ready to close" can trip the regex. The guard fires only on EndTurn (not mid-conversation), and the correction message is recoverable (one retry, then accept). Acceptable cost.

**Tests** (inline in `agent.rs`, `#[cfg(test)] mod tests`):
- `test_detect_milestone_close_claim_with_patch_passes` — text has claim AND argv has PATCH → returns None.
- `test_detect_milestone_close_claim_without_patch_caught` — text has claim AND no PATCH argv → returns the matched keyword.
- `test_detect_milestone_close_claim_case_insensitive` — "MILESTONE CLOSED" caught.
- `test_detect_milestone_close_claim_no_match_on_unrelated_close` — "PR closed" alone does NOT trigger (no "milestone" keyword).
- `test_detect_milestone_close_claim_readback_alone_not_sufficient` — only readback argv (`api ... --jq .state`), no PATCH → still triggers (readback without PATCH is not a close).
- **`test_dual_trigger_completion_and_milestone_close_emits_single_correction`** (F1 dual-trigger case): assistant text "Milestone#17 completed and closed", no `update_task_status` called, no PATCH argv. Drive the post-condition chain end-to-end (or a focused harness around guards #4 and #4b). Assert: **exactly one** correction message is pushed to `request.messages` for this response. Assert: the correction names the completion-claim violation (not the milestone-close one) — #4 fires first; #4b is short-circuited by the `continue` in the chain.
- **`test_milestone_close_fires_after_completion_claim_satisfied`** (companion to the dual-trigger test): same text, but `update_task_status` IS in `tools_called` and `completion_claim_retry_done = true`. Assert: #4 returns None (its trigger is unsatisfied), #4b fires, exactly one correction names the missing PATCH. Locks the multi-turn handoff semantics.

### Followups (file as new tickets, not in this PR)

- **mika#TBD-projects:** Project Workflow P5 close-out via `closeProjectV2` GraphQL. Same shape as M5 step 3 but different surface. Filed during /ce:work.
- **mika#TBD-completion-claim-extend:** Consider folding the new `detect_milestone_close_claim_without_patch` into a more general "verify-state-change-before-claiming" framework if a third instance appears (e.g., release publish, deploy completion). Not needed today.

## 4. Acceptance criteria (mapped to ticket)

| Ticket AC | This plan |
|---|---|
| AC1: prompt has explicit close step before marking local complete | Part B — Step M5 insert (new step 3a) |
| AC2: prompt has verify-readback step requiring `state == "closed"` | Part B — Step M5 sub-step 3b + 3c branch |
| AC3: integration test asserts close called once, verify called, success only on closed readback, escalation on open readback | Part C — eval scenarios C1 (happy path) + C2 (pre-fix regression) |
| AC4: regression replay of milestone#17's task sequence | Part C — C2 uses the milestone#17 response shape as the frozen fixture; full overnight task-sequence replay is reduced to the load-bearing turn-shape replay |
| AC5 (one-line memory note) | **Drop** — `feedback_verify_before_claiming.md` already captures the discipline; this fix structurally encodes it. PR description references the memory entry. If the architect pushes back, add the note. |

## 5. Scope boundaries

**In scope:**
- Prompt change to self-dev M5 (Part B).
- Eval scenarios C1 + C2 (Part C).
- Structural guard `detect_milestone_close_claim_without_patch` + tests (Part D).
- `blockedBy: mika#788` edge applied as a Phase-5 operator action (not committed in this PR).

**Out of scope:**
- Project-close workflow (different API surface — Projects v2 GraphQL). File followup ticket.
- Full overnight task-sequence replay fixture (representative turn-shape replay in C2 substitutes).
- Generalizing the new guard into a "verify-state-change-before-claiming" framework. Wait for a third instance.
- Dedicated `close_github_milestone` builtin tool. **Rejected by mika#788; not relitigated here** per Vincent's option (a).
- Adding `api` to `GH_ALLOWED_SUBCOMMANDS`. **Owned by mika#788, not this ticket.**
- Removing the speculative `milestone` and `project` entries from the allowlist. **Owned by mika#788, not this ticket.**

## 6. Implementation order (within the PR)

1. **Part B** first — prompt change. Standalone; can be exercised manually after mika#788 ships.
2. **Part D** second — structural guard + unit tests. Land alongside Part B so the guard's correction message references the prompt step.
3. **Part C** third — eval scenarios. Land last so they assert against the post-B/D state.

All three in one PR. Splitting risks the guard shipping before the prompt step it references exists, or the eval shipping with a "passing" fixture before the guard catches it.

## 7. Files touched

- `skills/bundled/self-dev/system_prompt.md` — M5 step 3 insertion + renumbering, notification template update, P5 NOTE.
- `crates/mika-agent/src/agent.rs` — new `detect_milestone_close_claim_without_patch` function (~25 lines), new firing site in the post-condition chain (~30 lines), `milestone_close_claim_retry_done` flag (~3 lines), inline tests (~80 lines).
- `crates/mika-agent/tests/eval/grounding_regressions/milestone_close.rs` — new scenario file.
- `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` — register scenario.
- `crates/mika-agent/tests/eval/grounding_regressions/fixtures/milestone_close_pre_fix.json` — frozen milestone#17 response shape.
- `crates/mika-agent/tests/eval/grounding_regressions/README.md` — add the new scenario + tag entry.

## 8. Open questions for architect (pass 2)

1. **Guard regex tolerance.** `(?i)\bmilestone\b.{0,80}\b(closed|close)\b` accepts "milestone X is closed" and "milestone X close-out". Does the architect want narrower (only "closed", past tense) or broader (also "shut", "done")? My lean: keep `closed|close` only — past-tense "closed" is the dominant incident pattern, infinitive "close" appears in plans like "we will close the milestone" which is also a claim worth guarding. Architect call.

2. **Readback enforcement in the guard.** Part D's guard requires the PATCH argv but NOT the readback argv. Reasoning: readback compliance is contractually load-bearing (the prompt step demands it) but tooling-detectable only via output-content inspection, not argv-pattern matching. Eval scenario C1 covers it. My lean: keep the guard PATCH-only, let the eval lock readback. Architect: is a parallel argv-shape check for `[api, ..., --jq, .state]` worth adding to the guard?

3. **`milestone_close_claim_retry_done` flag scope.** Standalone, same shape as `completion_claim_retry_done`. Alternative: reuse `completion_claim_retry_done` (a "completion claim of any shape was retried"). My lean: standalone — the retry budget for each failure class should be independent. If the agent fails both the completion-claim guard AND the milestone-close guard in the same turn, both should get a corrective re-prompt. Architect: ratify or push for shared flag.

4. **mika#788 PR-name reference in this plan.** I'm citing #788 throughout but its actual scope was settled in its own grooming (the brainstorm split Vincent mentioned). Should this plan link to mika#788's plan doc explicitly (not just the issue body) so the implementer can verify the allowlist change shape before /ce:work? My lean: yes — but #788 may not yet have a committed plan doc. If it does, cite the path. Architect: ratify.

## 9. Citations (now all paired with Phase 0 Pin verbatim)

- Site 1: `crates/mika-agent/src/skills/builtin_handlers.rs:1619-1630` — GH_ALLOWED_SUBCOMMANDS (current; will mutate via #788).
- Site 2: `crates/mika-agent/src/skills/builtin_handlers.rs:1770-1782` — `validate_gh_input` enforcement.
- Site 3: `skills/bundled/self-dev/system_prompt.md:266` — prompt's allowlist enumeration.
- Site 4: `skills/bundled/self-dev/system_prompt.md:515-528` — Step M5.
- Site 5: `crates/mika-agent/src/agent.rs:1216-1274` + `:4572-4598` — completion-claim guard precedent.
- `mika/CLAUDE.md` § "Post-Conditions (EndTurn Chain)" — post-condition #5 (fabricated action claim) is the closest structural sibling for the new guard.
- mika#788 issue body, "Committed decisions" section — verbatim cited in §2.
- mika#793 issue title verified via `gh issue view 793` 2026-05-16 — same-milestone sibling, parallel-safe sequencing.
- Memory `feedback_verify_before_claiming.md` — discipline this fix structurally encodes.
- Memory `feedback_prompt_enforcement_fragile.md` — guides the structural-guard choice (Part D) over pure prompt enforcement.
- Memory `feedback_dont_drift_umbrella_frame.md` — invoked by operator in rejecting option (b); cited here as the calibration record.

## 10. Revision history

- **Pass 1 (2026-05-16, this session):** drafted plan proposing dedicated `close_github_milestone` builtin tool. mika-arch session `b146aaa1-3b42-433a-b5ed-45d95366346a` returned **Disposition: ESCALATE** — F2 named mika#788's committed decision against dedicated milestone-close tools. Verified independently via `gh issue view 788`; architect's claim confirmed.
- **Operator intervention:** Vincent chose option (a) — block on mika#788, no override. Reasoning: mika#788 explicitly names mika#797 as the downstream "self-dev verify-post-state ticket"; override would compound umbrella-frame drift (`feedback_dont_drift_umbrella_frame`); phase-1 ordering inside milestone "Self-dev / dev-loop reliability v2" is mechanical with the blockedBy edge.
- **Revision 2 (this document):** plan rewritten to prompt-only + structural guard, with Phase 0 Pin per F1, mika#788/#793 sibling state documented per F3, and the dedicated-builtin shape explicitly rejected per the operator's call.
- **Pass 1 v2 (rev 2 first-pass review):** mika-arch session `56f72a32-ee3f-46e8-b2cd-eb5455f86ac2` returned **Disposition: ITERATE**. F1 — guard-interaction edge case (both `detect_completion_claim` and the new `detect_milestone_close_claim_without_patch` can match the same text); needed explicit semantics and a dual-trigger test. F2 — plan staged-not-committed; resolved by the spec's commit-on-ITERATE rule.
- **Revision 3 (this commit):** Part D's "Interaction with `detect_completion_claim`" subsection added with three-bullet semantics (one-guard-per-turn, independent retry flags, intentional ordering); two new tests added (`test_dual_trigger_completion_and_milestone_close_emits_single_correction` and `test_milestone_close_fires_after_completion_claim_satisfied`); plan committed to branch per spec.
