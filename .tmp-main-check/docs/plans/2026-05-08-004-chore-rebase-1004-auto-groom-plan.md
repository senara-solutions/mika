# Plan: Rebase PR #1004 (auto-groom) against main

- **Ticket:** mika issue#1035
- **Type:** chore (rebase + conflict resolution)
- **Branch:** `feat/996/autonomous-loop-auto-groom-every-ready` (existing PR branch — NOT a new branch)
- **PR:** #1004 (APPROVED, CI failing due to drift)
- **Date:** 2026-05-08

## Context

PR #1004 implements auto-groom-before-dispatch (mika#996): when a ready-labelled ticket lacks the Plan callout, mika-dev auto-grooms via dev-groom before dispatching dev-pilot. The PR was approved but its CI has been failing since 2026-05-06 due to drift from main (20 commits behind).

The branch carries 7 commits, of which 2 are duplicates of PRs already merged to main (#1002 dashboard tabs, #1003 release-pr fix). The 5 unique feature commits are:
1. `fee5bf0a` — plan doc (grooming artifact)
2. `9ef9d037` — plan doc GROOMED feedback
3. `490a79c9` — **core**: auto-groom dispatch in self-dev + dev-groom skill changes + CLAUDE.md
4. `b32eef74` — compound doc (grooming-as-dispatch-phase)
5. `96fb7462` — **core**: 3 additional eval tests (milestone-cascade, ESCALATE callback, 5-ticket cascade)

## Divergence analysis

### Main-side changes since divergence (20 commits)

Key changes that interact with the feature:

| Main commit | Area | Interaction risk |
|---|---|---|
| `ec6e3867` fix(autonomous-loop): engine-enforced queue advance (#991) | `self-dev/system_prompt.md` — rewrites callback entry point, adds heartbeat trigger | **HIGH** — same file, nearby sections |
| `0a130af3` fix(autonomous-loop): per-skill LLM override fires on webhook turns | `skills/bundled/self-dev/*` | LOW — different skill files |
| `875b6466` fix(task-engine): dispatch retry hygiene | `crates/mika-agent/src/task_engine/dispatcher.rs` | NONE — feature doesn't touch dispatcher.rs |
| `642a963c` fix(kg): single-consumer topology | KG crates | NONE — no overlap |
| Various eval test additions | `crates/mika-agent/tests/eval.rs` | **MEDIUM** — both add mod lines to sorted list |
| Release commits (v0.6.0–v0.9.0) | `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock` | LOW — feature doesn't touch these |

### Conflict file inventory

| File | Branch side | Main side | Classification |
|---|---|---|---|
| `crates/mika-agent/tests/eval.rs` | Adds `mod test_auto_groom_dispatch;` | Adds `mod test_callback_milestone_advance;`, `mod test_context_summary_inject;` | **Mechanical** — merge all mod lines alphabetically |
| `skills/bundled/self-dev/system_prompt.md` | Rewrites webhook dispatch Step 3 (grooming pre-flight), adds milestone Step 1.5 | Rewrites callback entry point (mika#991 engine contract), adds heartbeat trigger section | **Mechanical with care** — changes target different sections of the file, but proximity may cause conflict markers across section boundaries. Preserve both sets of changes. |
| `CLAUDE.md` | Adds autonomous-loop auto-groom section | Possibly different section updates | **Mechanical** — preserve both |
| `.github/workflows/ci.yml` | From duplicate #1003 commit | Same #1003 merged to main | **Auto-resolve** — identical content, rebase drops duplicate |
| `.github/workflows/release-pr.yml` | From duplicate #1003 commit | Same #1003 merged to main | **Auto-resolve** — rebase drops duplicate |
| `dashboard/src/App.tsx` | From duplicate #1002 commit | Same #1002 merged to main | **Auto-resolve** — rebase drops duplicate |
| `dashboard/src/pages/SessionDetail.tsx` | From duplicate #1002 commit | Same #1002 merged to main | **Auto-resolve** — rebase drops duplicate |
| `docs/solutions/*` | Compound docs from feature | Same docs on main (from merged PRs) | **Auto-resolve** — rebase drops duplicates |

### Semantic escalation triggers (AC#5)

The auto-groom path modifies:
1. **Webhook dispatch Step 3** — grooming pre-flight predicate change (`> - **Plan:**` → `Plan: docs/plans/` substring match)
2. **Milestone cascade Step 1.5** — pre-dispatch grooming check per child

Main's mika#991 changes modify:
1. **Callback entry point** — engine-enforced queue advance contract
2. **Post-callback permitted actions** — exhaustive list
3. **Heartbeat trigger** — stalled milestone queue detection

These are **orthogonal concerns** (dispatch-path grooming vs callback-path advancement). They touch different sections of `self-dev/system_prompt.md` and should compose without design tension. **Classification: mechanical, not semantic.**

## Phase 0 — Pre-rebase verification (mika-arch F1 + F2)

### 0a. Callback guard interaction analysis (F1 — mika#991 × mika#996)

The mika#991 `callback_milestone_advance` guard (in `agent.rs`) fires when ALL of:
- `callback_milestone_advance_trigger(&user_input_text)` — input is a callback with milestone context
- `extract_milestone_parent_id(&user_input_text)` — parent task ID extractable
- `!callback_milestone_advance_satisfied(parent_id, &all_tool_summaries)` — agent didn't advance

**Interaction analysis by path:**

| Auto-groom callback path | Has milestone parent? | Guard fires? | Agent action | Satisfies guard? |
|---|---|---|---|---|
| Webhook (Ready-Label Dispatch Step 3d) | NO — groom task is standalone (`?phase=groom` reference_url) | NO | N/A | N/A — guard doesn't apply |
| Milestone cascade (M4 Step 1.5c) | YES — child task has milestone parent | YES | GROOMED → `run_claude_pilot` (advance) | YES ✓ |
| Milestone cascade (M4 Step 1.5c) | YES | YES | ESCALATE → `update_task_status(blocked)` (halt) | YES ✓ |
| Milestone cascade (M4 Step 1.5c) | YES | YES | CRASH → retry `run_claude_pilot` or block | YES ✓ |

**Verdict: ORTHOGONAL — confirmed.** The auto-groom callback handlers satisfy the #991 guard in all milestone-context cases. The webhook path doesn't trigger the guard at all. The feature branch's prompt text at line 675 explicitly documents this: "Engine-guard implications: the milestone-cascade path does not flow through `webhook_ready_label_dispatch`. No new guard is needed; M4's existing dispatch-readiness checks already accept `dev-groom` as a valid `run_claude_pilot` skill." The self-dev prompt conflict is mechanical, not semantic. AC#5 escalation is NOT triggered.

### 0b. Test fixture API drift enumeration (F2)

Checked all interfaces imported by `test_auto_groom_dispatch.rs` against main-side changes:

| Import | Source | Changed on main? | Breaking? |
|---|---|---|---|
| `mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools}` | `tools/mod.rs` | NO | — |
| `mika_common::claude::ToolDefinition` | `claude.rs` | NO | — |
| `mika_common::llm::mock::*` | `llm/mock.rs` | NO | — |
| `super::assertions::*` | `tests/eval/assertions.rs` | NO | — |
| `super::harness::EvalHarness` | `tests/eval/harness.rs` | NO | — |

**Internal changes that DON'T affect test interfaces:**
- `skills/executor.rs` — `validate_dispatch_readiness` gained `tool_input: Option<&serde_json::Value>` parameter (mika#1011 deferred dispatch). Internal to executor, not imported by tests.
- `agent.rs` — callback_milestone_advance guard added (mika#991). Engine internals, not test-facing.

**Verdict: NO API drift.** All interfaces used by the auto-groom tests are unchanged on main. Build failures (if any) will come from textual conflicts in `eval.rs` mod list, not from type/signature changes.

## Execution steps

### Step 1: Pre-rebase verification

```bash
cd <worktree>
git log --oneline origin/main..HEAD  # Confirm 7 commits
git fetch origin main                # Ensure main ref is current
```

### Step 2: Interactive rebase with explicit duplicate drops (mika-arch NF1)

Use `git rebase -i` to explicitly drop the 2 duplicate commits rather than relying on auto-detection:

```bash
git rebase -i origin/main
```

In the interactive editor, mark these commits as `drop`:
- `a16642de fix(ci): release-pr workflow idempotency — Class C resolution (#1003)` — duplicate of main's `687fa0ad`
- `08c105cc feat(dashboard): sync session detail tabs with URL path segments (#676) (#1002)` — duplicate of main's `1706e683`

Keep the 5 unique feature commits as `pick`:
- `fee5bf0a docs(plans): groom mika issue#996 initial plan`
- `9ef9d037 docs(plans): apply mika-arch second-pass GROOMED feedback (mika issue#996)`
- `490a79c9 feat(self-dev): auto-groom ungroomed tickets before dispatch (#996)`
- `b32eef74 docs(solutions): compound — grooming as a phase of dispatch (#996)`
- `96fb7462 test(eval): add milestone-cascade, ESCALATE callback, and 5-ticket cascade tests (#996)`

Conflicts expected in:
- `eval.rs` (mod list merge)
- `self-dev/system_prompt.md` (section boundary conflicts)
- Possibly `CLAUDE.md`

**Note:** Since claude-pilot runs non-interactively, the implementer should use `GIT_SEQUENCE_EDITOR="sed -i '/a16642de/s/pick/drop/; /08c105cc/s/pick/drop/'"` or equivalent to automate the interactive rebase.

### Step 3: Conflict resolution (per file)

**`crates/mika-agent/tests/eval.rs`:**
- Merge all `mod` declarations. Maintain alphabetical order.
- Feature adds: `mod test_auto_groom_dispatch;`
- Main adds: `mod test_callback_milestone_advance;`, `mod test_context_summary_inject;`
- Result: all three present in sorted position.

**`skills/bundled/self-dev/system_prompt.md`:**
- Accept ALL of main's mika#991 changes (callback entry point rewrite, heartbeat trigger).
- Re-apply feature's changes on top:
  - Webhook dispatch Step 3 rewrite (grooming pre-flight with `Plan: docs/plans/` predicate)
  - Webhook dispatch auto-groom path (Steps 3a–3g)
  - GATE line update ("escalation" replacing "grooming rejection")
  - Milestone cascade Step 1.5 (grooming pre-flight per child)
- Per Phase 0a analysis: the two change sets target different sections and compose without design tension.

**`CLAUDE.md`:**
- Accept main's version. Re-apply feature's autonomous-loop auto-groom paragraph.

**Any other files:**
- Accept main's version (duplicate commits explicitly dropped).

### Step 4: Post-rebase verification (mika-arch NF2)

Five explicit gates before force-push — ALL must pass:

| # | Check | Command | PASS criterion |
|---|---|---|---|
| 1 | Compilation | `cargo build` | Exit 0, no errors |
| 2 | Tests | `cargo test` | Exit 0, all tests pass |
| 3 | Net diff preserved | `git diff origin/main..HEAD -- skills/bundled/self-dev/system_prompt.md crates/mika-agent/tests/eval/test_auto_groom_dispatch.rs skills/bundled/dev-groom/` | Feature's auto-groom additions present in diff |
| 4 | Commit count | `git log --oneline origin/main..HEAD \| wc -l` | Exactly 5 commits (7 original minus 2 dropped) |
| 5 | Prompt coherence | Manual read of self-dev prompt's auto-groom sections | Steps 3a–3g and Step 1.5 present, #991 callback entry point and heartbeat trigger also present, no orphaned cross-references |

### Step 5: Force-push (AC#4)

```bash
git push --force-with-lease origin feat/996/autonomous-loop-auto-groom-every-ready
```

PR #1004's CI re-runs automatically.

### Step 6: Verify CI

Monitor PR #1004 checks. If CI passes, the rebase is complete. If CI fails with new errors (not pre-existing), investigate — but that's outside this ticket's scope (escalate per AC#5).

## Risk assessment

- **Duplicate commit handling:** LOW risk. Explicit `drop` in interactive rebase eliminates auto-detection uncertainty.
- **self-dev prompt conflicts:** MEDIUM risk on mechanics, NONE on semantics. Per Phase 0a, the changes are verified orthogonal (dispatch grooming vs callback advancement). May produce large conflict markers due to proximity. Resolution strategy is clear: accept main, re-apply feature.
- **Type/API drift in test fixtures:** NONE per Phase 0b analysis. All test-facing interfaces are unchanged on main.
- **Force-push safety:** LOW risk. `--force-with-lease` prevents overwriting commits pushed by others since our last fetch.

## Out of scope

- Rebasing PR #1005 (sibling stale PR; separate ticket)
- Reimplementing auto-groom from scratch
- Any design changes to the auto-groom feature
