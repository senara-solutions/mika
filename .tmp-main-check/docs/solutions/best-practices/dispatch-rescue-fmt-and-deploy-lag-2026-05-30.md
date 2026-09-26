---
category: best-practices
module: self-dev
problem_type: best_practice
component: development_workflow
severity: high
tags: [dispatch-lib, cargo-fmt, deploy-lag, dirty-worktree, rescue, lefthook, mika-1282, mika-1296, mika-1310, mika-1336]
---

# Dispatch rescue cargo-fmt failure and deploy-lag wedge class

## Failure shape

The mika#1282 dirty-worktree rescue auto-commits pilot content when claude-pilot exits with uncommitted changes. The dominant rescue-failure class is pilot-authored Rust that was never `cargo fmt`-ed — the first rescue commit trips the lefthook `rust-fmt` pre-commit gate.

Pre-mika#1310, the commit output capture used `2>` (stderr only). Lefthook prints its summary and failure marks to stdout, not stderr. The capture was empty, the `grep -q "rust-fmt"` missed, and the code fell through to the "non-rustfmt" abort path — pilot content was abandoned silently.

mika#1310 fixed the capture by redirecting both stdout and stderr (`>file 2>&1`), and mika#1296 added the reactive `cargo fmt --all` retry loop.

## Deploy-lag class

`dispatch-lib.sh` is an engine-coupled bundled skill. It is **copy-deployed**: `make deploy` → `mika skills --agent <name> update` copies the source from `skills/bundled/_shared/` to `~/.mika/skills/_shared/`. This means main-merged ≠ live — a fix can sit on main for days while the deployed copy remains stale.

The 2026-05-29→30 wedge exposed this: the pre-#1310 copy was still deployed, so every rescue attempt hit the empty-capture bug even though the fix had been merged.

**Before concluding a substrate-skill bug persists after merge:**

1. `diff ~/.mika/skills/_shared/dispatch-lib.sh skills/bundled/_shared/dispatch-lib.sh`
2. Compare mtime vs the fix's merge date
3. Run `make deploy` if stale

## Fabrication note

mika-dev diagnosed "rust-analyzer not in PATH" as the root cause during one investigation. This was hallucinated — zero `rust-analyzer` references exist in dispatch-lib, lefthook config, or any handler script. The actual failure was the stdout-vs-stderr capture mismatch described above.

Cross-ref: the `mika-dev-llm-fabricates` compound doc family documents prior fabrication incidents.

## Resolution

mika#1336 adds **proactive `cargo fmt --all`** before the first rescue commit attempt. The block:

1. Checks if any staged files are `*.rs` via `git diff --cached --name-only | grep -q '\.rs$'`
2. Runs `cargo fmt --all` inside the worktree (best-effort — errors captured but do not block)
3. Re-stages formatted files with `git add -u` (update-only, scaffold exclusion)

This eliminates the dominant fmt-class rescue failure. The reactive retry (mika#1296 + mika#1310) is retained as belt-and-suspenders fallback.

### Cross-references

- `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md` — original dirty-worktree rescue pattern
- `docs/solutions/architecture-patterns/pilot-vs-substrate-contract-split-2026-05-25.md` — pilot vs substrate contract boundary (note: issue body references `cross-repo-patterns/` which is stale; the file lives under `architecture-patterns/`)
