# Plan: Proactive cargo fmt in post-flight rescue + deploy-lag compound doc

**Ticket:** mika issue#1336
**Type:** fix
**Component:** `skills/bundled/_shared/dispatch-lib.sh`, `docs/solutions/best-practices/`

## Context

The mika#1282 dirty-worktree rescue currently uses a **reactive** pattern: attempt commit → parse lefthook stdout for `rust-fmt` rejection → run `cargo fmt --all` → retry commit (mika#1296, mika#1310). This works but:

1. Depends on parsing lefthook stdout (`grep -q "rust-fmt"`) — fragile coupling to hook output format.
2. Pays cold-clippy compilation twice (~40s each) — once for the rejected commit, once for the retry.
3. The 2026-05-29→30 wedge class (deploy lag: `make deploy` not run after merge) exposed the fragility when the pre-#1310 copy was still deployed.

## Deliverables

### Deliverable 1: Proactive `cargo fmt --all` in dispatch-lib rescue block

**File:** `skills/bundled/_shared/dispatch-lib.sh`

**Location:** After line 483 (`RESCUED_FILES=$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9)`), before the rescue commit attempt at line 497.

**Change:** Insert a proactive formatting block that:

1. Checks if any staged files are `*.rs` via `git diff --cached --name-only | grep -q '\.rs$'`
2. If yes, runs `cargo fmt --all` inside `$WORKTREE_DIR`
3. Captures stderr but does not fail on error (formatting is best-effort optimization)
4. Re-stages all non-scaffold files after formatting (`git add -A -- ':!.claude/commands/'`)

**Exact insertion (between RESCUED_FILES computation and the commit attempt):**

```bash
# Proactive formatting (mika#1336): the dominant rescue-failure class is
# pilot-authored Rust that was never `cargo fmt`-ed, so the first commit
# trips the lefthook rust-fmt gate. Formatting up front makes the first
# commit succeed, halves wall-clock (one clippy compile, not two), and
# removes reliance on parsing lefthook stdout to detect a fmt rejection.
# The reactive rust-fmt retry below remains as belt-and-suspenders.
# Gated on staged *.rs so docs-only / non-Rust pilots don't pay cargo startup.
if git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9 | grep -q '\.rs$'; then
    PROACTIVE_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
    [ -n "$PROACTIVE_FMT_ERR" ] && echo "NOTE: proactive cargo fmt: ${PROACTIVE_FMT_ERR}" >&2
    git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9
fi
```

**Why this location:** After `RESCUED_FILES` is computed (line 483) and before the first `git commit` (line 497). The `RESCUED_FILES` variable is already captured, so re-staging after fmt doesn't lose the file list. The re-stage (`git add -A`) picks up any formatting changes that `cargo fmt` made to already-staged files.

**What stays unchanged:** The entire reactive branch at lines 517–569 (the `grep -q "rust-fmt"` → `cargo fmt` → retry path) remains as a fallback. If proactive formatting succeeds, the reactive branch never fires (the first commit succeeds). If proactive formatting somehow misses a case or a new lint is added to lefthook, the reactive branch catches it.

### Deliverable 2: Compound doc — deploy-lag wedge class

**File:** `docs/solutions/best-practices/dispatch-rescue-fmt-and-deploy-lag-2026-05-30.md`

**Frontmatter:**
```yaml
---
category: best-practices
module: self-dev
problem_type: best_practice
component: development_workflow
severity: high
tags: [dispatch-lib, cargo-fmt, deploy-lag, dirty-worktree, rescue, lefthook, mika-1282, mika-1296, mika-1310, mika-1336]
---
```

**Content sections:**

1. **Failure shape** — Auto-rescue commit rejected by lefthook `rust-fmt` gate. Pre-mika#1310, the capture used `2>` (stderr only); lefthook prints to stdout → capture was empty → grep missed → fallthrough to "non-rustfmt" abort → pilot content abandoned silently.

2. **Deploy-lag class** — `dispatch-lib.sh` is a **copy-deployed** bundled skill. `make deploy` → `mika skills --agent <name> update` copies the source to `~/.mika/skills/_shared/`. main-merged ≠ live for `skills/bundled/**` — the fix can sit on main for days while the deployed copy remains stale. Before concluding a substrate-skill bug persists:
   - `diff ~/.mika/skills/_shared/dispatch-lib.sh skills/bundled/_shared/dispatch-lib.sh`
   - Compare mtime vs the fix's merge date
   - Run `make deploy` if stale

3. **Fabrication note** — mika-dev's "rust-analyzer not in PATH" diagnosis was hallucinated. Zero `rust-analyzer` references exist in dispatch-lib, lefthook config, or any handler script. Cross-ref existing `mika-dev-llm-fabricates` compound doc family.

4. **Resolution** — Proactive `cargo fmt --all` (mika#1336) eliminates the dominant fmt-class rescue failure. Reactive retry (mika#1296 + mika#1310) retained as fallback. Cross-references:
   - `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`
   - `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md`

## Implementation order

1. Write the compound doc (Deliverable 2) — no code dependencies, documents the investigation.
2. Insert the proactive fmt block in dispatch-lib.sh (Deliverable 1).
3. Run `cargo fmt` and `cargo clippy` (the dispatch-lib change is shell, but verify no Rust side-effects).
4. Manual verification: in a test worktree, stage unformatted `.rs` files and confirm the proactive path fires before commit.

## Risk assessment

**Low risk.** The change is additive — a `cargo fmt --all || true` before an existing commit path. The reactive fallback is untouched. Failure modes:

- `cargo fmt` errors (missing toolchain, compilation errors): captured and logged as NOTE, does not block the commit attempt. The reactive path catches the subsequent hook failure.
- Non-Rust worktrees: gated on `grep -q '\.rs$'`, so no cargo startup cost for non-Rust pilots.
- Re-staging after fmt: uses the same `':!.claude/commands/'` exclusion as the initial stage, so scaffold paths remain excluded.

## Files changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Insert proactive `cargo fmt --all` block (~8 lines) after RESCUED_FILES, before first rescue commit |
| `docs/solutions/best-practices/dispatch-rescue-fmt-and-deploy-lag-2026-05-30.md` | New compound doc (~80 lines) |
