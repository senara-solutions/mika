---
issue: 1682
type: fix
date: 2026-06-30
---

# Plan — fix(mika-dev): mika-dev auto-un-drafts wip-rescue PRs (mika#1613 layer-2 bypass) (mika#1682)

## Problem

mika#1613's qa-webhook guard chain (Guard 1 metadata flag, Guard 2 `isDraft AND ^wip\(`) enforces an operator-review contract on dispatch-lib rescue draft PRs: they stay draft until the operator un-drafts. This contract is being silently bypassed in production by a separate autonomous-loop step where mika-dev (the LLM) auto-promotes the rescue PR — renames the title from `wip(...)` to `fix(...)` AND un-drafts — likely as part of its webhook-event handling.

**Live evidence captured 2026-06-30 ~15:07Z on mika#PR1681:**

- `15:03:43Z` PR opened by dispatch-lib (mika#1383 path, non-draft per mika#1679's documented bypass)
- `15:06:32Z` operator-CC `convert_to_draft` + added `wip-rescue` label (restore mika#1613 contract)
- `15:07:55Z` `mika-platform-dev` (mika-dev agent) **renamed** title `wip(mika#1663): impl staged...` → `fix(mika#1663): skill-review variant path uses provider naming`
- `15:07:56Z` `mika-platform-dev` **ready_for_review** (un-drafted)

The two operations are 1 second apart = single mika-dev LLM turn calling `gh pr edit --title` + `gh pr ready` in succession. Grep of `skills/bundled/` for `gh pr ready` / `--undo` returns zero hardcoded calls — confirming this is LLM-driven, not a hardcoded skill step. Per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, prompt-only contracts don't bind across model classes — confirming why mika#1613's prompt-level guards aren't holding.

## Architectural lineage

- mika#1613 (CLOSED, PR#1677) — original wip-rescue operator-review contract; this issue documents its layer-2 bypass.
- mika#1679 (GROOMED + ready) — sibling fix at dispatch-lib level (closes layer-1 bypass: mika#1383 path opens non-draft).
- mika#525 — tool-boundary guard pattern (`validate_dispatch_readiness`) — the architectural precedent this fix mirrors.
- mika#1196 — `validate_qa_review_gh_scope` — skill-scoped `run_gh` validator (the engine-side validation pattern).
- mika#1167 — `validate_gh_api_scope` — `gh api` per-method gating matrix (same engine-side gating shape).
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — why this fix MUST be structural, not prompt-only.

## Fix shape (engine-side tool-boundary guard)

Add a new validator `validate_pr_ready_undraft_scope` in `crates/mika-agent/src/skills/builtin_handlers.rs` (alongside the existing `validate_qa_review_gh_scope` + `validate_gh_api_scope`), called from `run_gh` before subprocess spawn.

**Detection:**
1. Identify ready-promoting `run_gh` calls:
   - `gh pr ready <N>` (without `--undo`) — explicit un-draft.
   - `gh pr edit <N> --title <T>` paired with rename FROM `wip(...)` (heuristic: hard to detect post-rename, but we can check the PR's CURRENT title is `wip(...)` and reject the rename).
2. For each detected call, extract the PR number from args (positional or via `--head` flag).
3. Call `gh pr view <N> --json isDraft,labels,commits` (synchronous, 30s timeout, reuses `ctx.github_token`).
4. Check if the PR matches the wip-rescue signature:
   - `isDraft: true` AND `commits[-1].messageHeadline` matches `^wip\(` (commit-prefix check), OR
   - Has `wip-rescue` label.
5. If wip-rescue: reject with structured error referencing mika#1613's operator-review contract.
6. If not wip-rescue (or PR fetch failed — fail-open per existing pattern): allow.

**Rejection error shape (matches `validate_qa_review_gh_scope` style):**

```
Cannot un-draft / rename wip-rescue PR #<N>. This PR was opened by dispatch-lib's
post-flight recovery (mika#1282 or mika#1383). The wip-rescue draft state is the
operator-review gate per mika#1613 — the operator must un-draft this PR manually
after reviewing the rescued work.

To proceed: leave the PR draft. The operator will review and promote it.
```

**Composition with mika#1679:** mika#1679 closes the layer-1 bypass (dispatch-lib opening non-draft). This fix (mika#1682) closes the layer-2 bypass (mika-dev re-undrafting). Both fixes together restore mika#1613's contract.

**Composition with mika#1196 / mika#1167:** runs in the same chain as the existing scope validators. Order: global allowlist → qa-review skill-scope → gh-api method matrix → **wip-rescue ready-promote scope (new)**. The new validator only fires on `pr ready` / specific `pr edit` shapes; pass-through for all other calls.

## Implementation outline

1. **New constant + validator function** in `crates/mika-agent/src/skills/builtin_handlers.rs` near line 1904 (alongside `validate_qa_review_gh_scope`):

   ```rust
   /// Validate `gh pr ready` / `gh pr edit --title` against wip-rescue contract (mika#1682).
   /// Rejects un-draft attempts on PRs whose head commit is `wip(...)` or whose labels
   /// include `wip-rescue` — mika#1613's operator-review contract requires manual un-draft.
   fn validate_pr_ready_undraft_scope(args: &[String], ctx: &ToolContext<'_>) -> Result<(), ToolOutput> {
       // Detect ready-promoting shapes
       let is_ready_promote = matches!(
           (args.first().map(String::as_str), args.get(1).map(String::as_str)),
           (Some("pr"), Some("ready"))
       ) && !args.contains(&"--undo".to_string());

       let is_pr_edit_title = matches!(
           (args.first().map(String::as_str), args.get(1).map(String::as_str)),
           (Some("pr"), Some("edit"))
       ) && args.iter().any(|a| a == "--title");

       if !is_ready_promote && !is_pr_edit_title { return Ok(()); }

       // Extract PR number (positional arg after subcommand+verb)
       let pr_num = args.iter().skip(2).find(|a| a.parse::<u32>().is_ok());
       let Some(pr_num) = pr_num else { return Ok(()); };  // fail-open if no PR num parseable

       // Synchronous gh pr view to fetch isDraft, labels, head commit headline
       // (reuses ctx.github_token; 30s timeout; fail-open on API error)
       let wip_rescue = check_wip_rescue_status(pr_num, ctx);
       match wip_rescue {
           Ok(true) => Err(ToolOutput::error(/* structured error per §Fix shape */)),
           Ok(false) | Err(_) => Ok(()),  // not wip-rescue OR fail-open on fetch error
       }
   }
   ```

   `check_wip_rescue_status` helper: synchronous `gh pr view` shell-out, parse JSON, return `Ok(true)` if (isDraft=true AND head-commit matches `^wip\(`) OR labels contain `wip-rescue`.

2. **Wire into `run_gh`** (line 2025 area). Add the call alongside existing validators:

   ```rust
   if let Err(err) = validate_qa_review_gh_scope(&gh_args.args, ctx) { return err; }
   if let Err(err) = validate_pr_ready_undraft_scope(&gh_args.args, ctx) { return err; }  // NEW
   // ... existing gh_api_invocation logic unchanged
   ```

3. **Audit event emission** — on rejection, emit `pr_ready_undraft_blocked` audit event mirroring `gh_api_invocation` shape:

   ```rust
   info!(
       target: "audit",
       event = "pr_ready_undraft_blocked",
       agent_id = %ctx.agent_id(),
       pr_number = pr_num,
       reason = "wip_rescue_contract",
       repo = %extract_repo_from_args(&args).unwrap_or("unknown"),
   );
   ```

4. **Unit tests** in the existing test mod near line 6647:
   - `test_validate_pr_ready_undraft_blocks_wip_rescue_label` — PR has wip-rescue label → reject.
   - `test_validate_pr_ready_undraft_blocks_wip_commit` — head commit `^wip\(` → reject.
   - `test_validate_pr_ready_undraft_allows_normal_pr` — non-draft, no label, normal commit → pass.
   - `test_validate_pr_ready_undraft_fail_open_on_api_error` — gh pr view fails → pass (don't break legitimate flows).
   - `test_validate_pr_edit_title_blocks_rename_on_wip_rescue` — `pr edit <N> --title <T>` on wip-rescue PR → reject.
   - `test_validate_passes_pr_ready_undo` — explicit `--undo` (convert TO draft) → pass.

5. **Defense-in-depth — skill prompt instruction (Layer B)**: add explicit instruction to `self-dev-webhook-qa/system_prompt.md`:

   ```
   ## Wip-rescue contract (mika#1613 / mika#1682)
   Do NOT call `gh pr ready` or `gh pr edit --title` on any PR matching the
   wip-rescue signature: `wip-rescue` label OR head commit starts with `wip(`.
   The operator must un-draft these PRs manually after reviewing the rescued
   work. Engine-side guard mika#1682 will reject the tool call if attempted —
   this instruction is a prompt-level reinforcement to avoid hitting the guard.
   ```

   Prompt-only won't bind across model classes (per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`), but adds context for the model to make the right decision when the engine guard fires.

## Acceptance criteria

- **AC1** — Hard evidence captured in issue body (DONE — mika#PR1681 timeline).

- **AC2** — Engine-side validator `validate_pr_ready_undraft_scope` implemented in `crates/mika-agent/src/skills/builtin_handlers.rs`, wired into `run_gh` after the existing `validate_qa_review_gh_scope` chain. Rejects `gh pr ready <N>` (without `--undo`) AND `gh pr edit <N> --title <T>` when target PR matches wip-rescue signature (isDraft=true + head commit `^wip\(` OR `wip-rescue` label). Structured error references mika#1613 contract. Fail-open on `gh pr view` API errors.

- **AC3** — `pr_ready_undraft_blocked` audit event emitted on rejection with `agent_id`, `pr_number`, `reason`, `repo` fields.

- **AC4** — Unit tests (6 cases per §Implementation outline §4) cover happy path + label match + commit match + fail-open + edit-title path + explicit-undo pass-through.

- **AC5 (architect F1 expanded)** — Skill prompt instruction (defense-in-depth Layer B per §Implementation §5) added to **all webhook-handler skill prompts**, not just qa: `self-dev-webhook-qa/system_prompt.md`, `self-dev-webhook-ci/system_prompt.md`, `self-dev-webhook-ready-label/system_prompt.md`, and `self-dev-callback/system_prompt.md`. The offending un-draft turn could fire from any webhook-event handler; covering one skill leaves the others exposed.

- **AC6** — Post-deploy smoke: the next 3 dispatch-lib-rescue PRs (mika#PR1681 itself + any new ones from queue) should NOT be auto-un-drafted by mika-dev. Verified via `gh api repos/.../issues/<N>/timeline` filtered for `ready_for_review` events from `mika-platform-dev` actor — count should be 0 across the smoke window.

## Out of scope

- **Auto-promote of non-wip-rescue PRs.** mika-dev legitimately un-drafts its own non-rescue PRs after CI green. The guard discriminates on the wip-rescue signature; only those are blocked.
- **mika#1679 dispatch-lib fix.** That's layer-1; this is layer-2. Composes cleanly — both needed for full contract restoration.
- **Reverting glm-5.2 swap.** Calibration-discipline-gated; orthogonal axis.
- **Broader autonomous-loop PR-state management policy reform.** This ticket scopes to closing the specific wip-rescue contract bypass.

## Files involved

- `crates/mika-agent/src/skills/builtin_handlers.rs` — validator function + wire-in + tests
- `skills/bundled/self-dev-webhook-qa/system_prompt.md` — Layer B defense-in-depth (architect F1)
- `skills/bundled/self-dev-webhook-ci/system_prompt.md` — Layer B defense-in-depth (architect F1)
- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md` — Layer B defense-in-depth (architect F1)
- `skills/bundled/self-dev-callback/system_prompt.md` — Layer B defense-in-depth (architect F1)
- No schema migration; no skill manifest changes

## Verification

- **Static:** `cargo clippy -p mika-agent` clean. `cargo test -p mika-agent builtin_handlers::tests::validate_pr_ready_undraft*` covers all 6 unit-test cases.
- **Integration:** existing `qa-review`/`run_gh` integration tests stay green (no regression on legitimate `gh pr` calls).
- **Live (AC6):** post-deploy, the next 3 wip-rescue PRs hold draft state. Confirmed by timeline query.

## References

- mika#1613 (CLOSED) — original wip-rescue operator-review contract
- mika#1679 (GROOMED + ready) — sibling fix at dispatch-lib level (layer 1)
- mika#1196 — `validate_qa_review_gh_scope` pattern
- mika#1167 — `validate_gh_api_scope` matrix pattern
- mika#525 — tool-boundary guard pattern precedent
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate.md` — why this fix is structural
- mika#PR1681 — live evidence of layer-2 bypass (timeline above)
- Operator screenshot / push notification 2026-06-30 ~15:09Z
