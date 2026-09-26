# Plan: Rebase PR #1005 (dev-handsoff) against main

- **Ticket:** mika issue#1037
- **Type:** chore (rebase + conflict resolution)
- **Branch:** `feat/967/skills-add-dev-handsoff-bundled-skill-v0` (existing PR branch — NOT a new branch)
- **PR:** #1005 (APPROVED, CI failing due to drift)
- **Date:** 2026-05-08

## Context

PR #1005 implements the dev-handsoff bundled skill v0.1 (mika#967): a keyword-triggered, prompt-only, artifact-only skill that writes structured handsoff logs to `~/.mika/handsoff/` at end-of-run. The PR was approved but its CI has been failing since 2026-05-06 due to drift from main.

The branch carries 7 unique commits — all feature commits, no duplicates of merged PRs (unlike sibling #1004 which had 2 duplicates to drop):
1. `ee0f0c66` — plan doc (grooming artifact)
2. `7a889393` — plan doc first-pass review
3. `4f891883` — plan doc GROOMED feedback
4. `04d08c30` — **core**: skill.toml + system_prompt.md for dev-handsoff
5. `5960b694` — CLAUDE.md listings update
6. `4b9559da` — compound doc (required_tools resolution)
7. `c5ecb0d6` — fix duplicate mika-arch-groom-ticket line in CLAUDE.md

Merge base: `687fa0ad` (fix(ci): release-pr workflow idempotency — shared commit on both branches).

## Divergence analysis

### Main-side changes since divergence (20 commits)

Key changes that interact with the feature:

| Main commit | Area | Interaction risk |
|---|---|---|
| `9281b2ce` feat(self-dev): auto-groom ungroomed tickets before dispatch (#996) (#1004) | `CLAUDE.md`, `crates/mika-agent/CLAUDE.md`, skills/bundled/dev-groom/*, self-dev/* | **MEDIUM** — both branches modify CLAUDE.md skill listings |
| Various release commits (v0.6.0–v0.9.1) | `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock` | **NONE** — feat/967 doesn't touch these |
| `ec6e3867` fix(autonomous-loop): engine-enforced queue advance (#991) | `agent.rs`, self-dev prompt | **NONE** — feat/967 doesn't touch self-dev or agent.rs |
| `642a963c` fix(kg): single-consumer topology (#800) | KG crates | **NONE** — no overlap |
| Various eval test additions | `crates/mika-agent/tests/eval.rs` | **NONE** — feat/967 doesn't add eval tests |

### Conflict file inventory

| File | Branch side | Main side | Classification |
|---|---|---|---|
| `CLAUDE.md` | Adds dev-handsoff to skill directory listing + fixes duplicate mika-arch-groom-ticket line | Adds auto-groom section, dev-groom skill, multiple section updates across 20 commits | **Mechanical** — accept main's version, re-apply dev-handsoff listing insertion and duplicate-line fix |
| `crates/mika-agent/CLAUDE.md` | Bumps engine-coupled skill count from 15→16, adds dev-handsoff to list | Multiple updates from 20 commits (new skills, sections, conventions) | **Mechanical** — accept main's version, re-apply skill count bump and dev-handsoff entry |
| `docs/plans/*` | Adds `2026-05-06-002-feat-dev-handsoff-bundled-skill-v0-plan.md` | Adds different plan files | **No conflict** — new file, no overlap |
| `docs/solutions/*` | Adds `bundled-skill-required-tools-resolution-2026-05-06.md` | Adds different solution files | **No conflict** — new file, no overlap |
| `skills/bundled/dev-handsoff/*` | Adds `skill.toml` + `system_prompt.md` | Not present on main | **No conflict** — new directory |

### Feature commit file inventory (Phase 0 verification)

All 7 feature commits touch exactly these 6 files:

| File | Commits | Category |
|---|---|---|
| `skills/bundled/dev-handsoff/skill.toml` | `04d08c30` | **New skill directory** — no main-side counterpart |
| `skills/bundled/dev-handsoff/system_prompt.md` | `04d08c30` | **New skill directory** — no main-side counterpart |
| `docs/plans/2026-05-06-002-feat-dev-handsoff-bundled-skill-v0-plan.md` | `ee0f0c66`, `7a889393`, `4f891883` | **New file** — no main-side counterpart |
| `docs/solutions/best-practices/bundled-skill-required-tools-resolution-2026-05-06.md` | `4b9559da` | **New file** — no main-side counterpart |
| `CLAUDE.md` | `5960b694`, `c5ecb0d6` | **Conflict** — skill directory listing section (`## Directory Structure > skills/bundled/`) |
| `crates/mika-agent/CLAUDE.md` | `5960b694` | **Conflict** — engine-coupled skill count + skill list section (`## Skills System`) |

**Verified absent from feature commits:** No Rust source files (`*.rs`), no `Cargo.toml`/`Cargo.lock`, no `self-dev/` files, no `agent.rs`, no `eval.rs`, no KG files, no dispatcher files. The feature is confined to the new skill directory + documentation listings.

**CLAUDE.md conflict sections identified:**
- `CLAUDE.md` conflicts occur in the `## Directory Structure > skills/bundled/` tree listing (where both branches add skill entries) and potentially in the duplicate mika-arch-groom-ticket line that commit `c5ecb0d6` fixes. These are inventory sections, not behavioral instruction sections.
- `crates/mika-agent/CLAUDE.md` conflicts occur in the `## Skills System` section (skill count and skill list). This is a documentation inventory, not a behavioral contract.

Neither conflict section contains dispatch protocol, callback semantics, or engine behavior instructions that main-side changes (#991, #996, #800) also modified.

### Semantic escalation triggers (AC#5)

The dev-handsoff skill is **fully self-contained**: prompt-only, artifact-only, no cross-dependencies with any main-side changes. Per the file inventory above, the 7 feature commits touch zero Rust source files, zero engine files, and zero shared skill files. The only main-side overlap is in CLAUDE.md inventory sections.

**Classification: mechanical, not semantic.** AC#5 escalation is NOT triggered.

### Comparison with sibling mika#1035 (PR #1004)

| Dimension | mika#1035 (PR #1004) | mika#1037 (PR #1005) |
|---|---|---|
| Duplicate commits to drop | 2 (#1002, #1003) | 0 |
| Rust source conflicts | eval.rs mod list, self-dev prompt | None |
| CLAUDE.md conflicts | Yes (auto-groom section) | Yes (skill listing only) |
| Semantic risk | Medium (self-dev prompt orthogonality) | None (self-contained new skill) |
| Build surface | Cargo.toml/Cargo.lock + eval tests | build.rs discovery only (new skill dir) |

This rebase is **simpler** than the sibling: no duplicate drops, no Rust source conflicts, no semantic risk assessment needed.

## Execution steps

### Step 1: Pre-rebase verification

In the worktree at the PR branch:
```bash
git fetch origin main
git fetch origin feat/967/skills-add-dev-handsoff-bundled-skill-v0
git checkout feat/967/skills-add-dev-handsoff-bundled-skill-v0
git log --oneline origin/main..HEAD  # Confirm 7 commits
```

### Step 2: Rebase onto main

No interactive rebase needed — all 7 commits are unique feature commits, no duplicates to drop:
```bash
git rebase origin/main
```

Conflicts expected only in:
- `CLAUDE.md`
- `crates/mika-agent/CLAUDE.md`

### Step 3: Conflict resolution (per file)

**`CLAUDE.md`:**
- Accept main's version as the base (it has 20 commits of updates).
- Re-apply feat/967's changes on top:
  - Add `├── dev-handsoff/` to the bundled skill directory listing (alphabetically, between `dev-groom/` and `dev-pilot/`)
  - Verify the duplicate mika-arch-groom-ticket line fix is still applicable (may be moot if main already resolved it)

**`crates/mika-agent/CLAUDE.md`:**
- Accept main's version as the base.
- Re-apply feat/967's changes: bump engine-coupled skill count (check main's current count and add 1), add `dev-handsoff` entry to the skill list with ticket reference #967.

### Step 4: Post-rebase verification

Five explicit gates before force-push — ALL must pass:

| # | Check | Command | PASS criterion |
|---|---|---|---|
| 1 | Compilation | `cargo build` | Exit 0, no errors (build.rs discovers new `dev-handsoff/` directory) |
| 2 | Tests | `cargo test` | Exit 0, all tests pass |
| 3 | Net diff preserved | `git diff origin/main..HEAD -- skills/bundled/dev-handsoff/ docs/plans/2026-05-06-002-feat-dev-handsoff-bundled-skill-v0-plan.md docs/solutions/best-practices/bundled-skill-required-tools-resolution-2026-05-06.md` | All dev-handsoff feature files present in diff |
| 4 | Commit count | `git log --oneline origin/main..HEAD \| wc -l` | Exactly 7 commits |
| 5 | Prompt + listing coherence | Read `skills/bundled/dev-handsoff/system_prompt.md`, `CLAUDE.md`, `crates/mika-agent/CLAUDE.md` | (a) `system_prompt.md` is present and byte-identical to pre-rebase version (no unintended conflict resolution artifacts); (b) dev-handsoff listed in both CLAUDE.md skill directory trees; (c) skill count in `crates/mika-agent/CLAUDE.md` is main's count + 1; (d) no duplicate listing lines |

### Step 5: Force-push (AC#4)

```bash
git push --force-with-lease origin feat/967/skills-add-dev-handsoff-bundled-skill-v0
```

PR #1005's CI re-runs automatically.

### Step 6: Verify CI

Monitor PR #1005 checks. If CI passes, the rebase is complete. If CI fails with new errors (not pre-existing), investigate — but that's outside this ticket's scope (escalate per AC#5).

## Risk assessment

- **No duplicate commit handling needed:** Simplest rebase case — all commits are unique.
- **CLAUDE.md conflicts:** LOW risk. Textual merge of skill directory listing. Well-bounded.
- **Build verification:** LOW risk. The `build.rs` auto-discovers `skills/bundled/dev-handsoff/` as a new directory. No Cargo.toml/Cargo.lock changes on this branch — cargo just picks up the new skill manifest.
- **Force-push safety:** LOW risk. `--force-with-lease` prevents overwriting commits pushed by others since our last fetch.
- **Semantic conflicts:** NONE. dev-handsoff is a fully self-contained prompt-only skill with no engine dependencies beyond the standard `write_agent_file` tool. No main-side changes affect its design.

## Institutional learnings applied

- **build.rs code generation hygiene** (`docs/solutions/best-practices/build-rs-code-generation-hygiene.md`): Run full `cargo build` (not just check) after rebase to verify build.rs correctly discovers the new `dev-handsoff/` skill directory.
- **Stale-base conflicting PRs** (`docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md`): Follow the established rebase guard pattern — fetch, check behind-count, rebase, capture conflicts on failure.
- **Force-push on owned branches** (`docs/solutions/ci-cd/release-automation-chronic-drift-2026-04-23.md`): `--force-with-lease` is safe on single-author feature branches.

## Out of scope

- Modifying the dev-handsoff skill's design
- Closing #1005 in favor of fresh reimplementation
- Any changes beyond rebase + conflict resolution
