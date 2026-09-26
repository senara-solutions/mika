# Plan: fix(commands): restore polymorphic `/mika` for mika sub-repo — mika#1255

## Problem

`mika/.claude/commands/mika.md` is a verbatim md5-identical copy of the meta-repo's cross-repo dispatcher at `mika-platform/.claude/commands/mika.md` (260 lines, md5 `7f1709767ba3033448a75195fce6ef3f`). This violates the documented "Slash Command Isolation" principle in `mika-platform/CLAUDE.md`.

The cross-repo dispatcher contains logic that only makes sense from the meta-repo — keyword inference, brainstorm dispatch, free-text dispatch, plan-as-input dispatch, cross-repo routing, and a `SCRIPTS_DIR` resolution path (`git rev-parse --git-common-dir`) that resolves to the wrong directory when invoked from a mika sub-repo worktree. This breaks the autonomous loop's dev-pilot dispatch on mika sub-repo issues.

## Evidence

- md5sum identity confirmed in ticket body
- mika-cloud (72 lines), mika-skills (72 lines), and claude-pilot-py (78 lines) all have correct polymorphic shapes
- Only mika sub-repo has the verbatim-copy bug
- Git log shows three commits that incrementally pulled mika's `/mika` toward the meta-repo shape

## Approach

Rewrite `mika/.claude/commands/mika.md` to follow the proven polymorphic sub-repo pattern established by mika-cloud, mika-skills, and claude-pilot-py. The new file will be ~80-100 lines (vs current 260).

## Changes

### File: `.claude/commands/mika.md` (REWRITE)

**Delete** all 260 lines of the current file. **Replace** with a polymorphic sub-repo `/mika` command modeled on `mika-cloud/.claude/commands/mika.md`, with these mika-specific adaptations:

#### 1. Frontmatter + scope header

```yaml
---
name: mika
description: Mika core development workflow with quality gates
argument-hint: "[feature description]"
disable-model-invocation: true
---
```

Plus `<!-- SCOPE: mika repo ONLY. Do NOT copy this to the meta-repo or other sub-repos. -->`.

#### 2. Issue linking

Same pattern as mika-cloud — parse `$ARGUMENTS` for `#<number>`, fetch from `senara-solutions/mika`.

#### 3. Worktree isolation

Identical structure to mika-cloud:

1. **Parse branch** — priority: explicit `branch:` prefix → issue body callout → walk-up `derive-branch-name` script.
2. **Skip if no branch/args.**
3. **Detect existing worktree (MANDATORY)** — `git rev-parse --git-dir` vs `--git-common-dir` check. If already in worktree, STOP setup, proceed to pipeline.
4. **Sync main** — `git fetch origin main:main` with fallback.
5. **Create worktree** — via `derive-worktree-path --branch "$BRANCH" --repo mika`.

The walk-up `SCRIPTS_DIR` resolution pattern (traverse `$(pwd)` upward looking for `scripts/derive-branch-name`) replaces the broken `git rev-parse --git-common-dir` approach. This works from both `<meta>/mika/` (main checkout) and `<meta>/.claude/worktrees/<slug>/mika/` (worktree).

#### 4. Pipeline

Same CE pipeline as other sub-repos, with mika's existing `verify-pipeline.sh`:

1. `/ce:plan $ARGUMENTS`
2. `/ce:work`
3. `/ce:review`
4. `/compound-engineering:resolve_todo_parallel`
5. `/ce:compound`
6. `bash scripts/verify-pipeline.sh` — retry loop on failure
7. `gh pr create --repo senara-solutions/mika` with `Closes #<number>` if issue-linked

**No mika-specific build/lint gates in the command itself.** The Rust build (`cargo build`, `cargo test`, `cargo clippy`) is handled by `/ce:work` and `/ce:review` — they read `CLAUDE.md` for the repo's build commands. Adding them here would duplicate what the CE pipeline already does. This matches mika-cloud and mika-skills, which also don't have tech-stack-specific gates in their `/mika` commands (only claude-pilot-py adds `ruff`/`mypy`/`pytest` because it doesn't use CE's built-in Python detection).

#### 5. Cleanup

Same as all sub-repos: do NOT remove worktree, output `<promise>DONE</promise>`.

### What gets removed

All of the following meta-repo-only concerns are deleted:

- "Operating principle: start with WHY" preamble (meta-repo orchestrator concern)
- Keyword inference rules (cross-repo routing)
- Branch-name derivation § with `git rev-parse --git-common-dir` bash snippet (broken from sub-repo)
- Brainstorm dispatch section
- Direct dispatch with sub-repo/self-targeting routing
- Free-text dispatch section
- Plan-as-input dispatch section
- Self-targeting pipeline section
- Step 1: Gather context (backlog evaluation)
- Step 2: Evaluate and present
- Step 3: Dispatch (cross-repo routing)

### What stays unchanged

- `mika-platform/.claude/commands/mika.md` — the meta-repo cross-repo dispatcher. Not touched. md5 must remain `7f1709767ba3033448a75195fce6ef3f`.

## Verification

1. `md5sum .claude/commands/mika.md` ≠ `7f1709767ba3033448a75195fce6ef3f`
2. `wc -l .claude/commands/mika.md` ≤ 120
3. `grep -c "cross-repo dispatcher\|keyword inference\|brainstorm dispatch\|Sub-repo path" .claude/commands/mika.md` = 0
4. `grep -c "SCOPE: mika repo ONLY" .claude/commands/mika.md` = 1
5. `grep -c "derive-branch-name" .claude/commands/mika.md` ≥ 1 (walk-up pattern present)
6. `grep -c "git rev-parse --git-common-dir" .claude/commands/mika.md` = 0 (broken pattern removed)
7. Meta-repo unchanged: `md5sum ../../.claude/commands/mika.md` = `7f1709767ba3033448a75195fce6ef3f` (from worktree) or verify via `git -C <meta> show HEAD:.claude/commands/mika.md | md5sum`

## Risk assessment

**Low risk.** This is a documentation/command file change — no Rust code, no schema migration, no runtime behavior change. The new shape is proven across three other sub-repos. The only risk is a copy-paste error in the walk-up pattern or repo name; verification step 5-6 catches that.

## Estimated scope

Single file rewrite, ~80-100 lines replacing 260. One commit.
