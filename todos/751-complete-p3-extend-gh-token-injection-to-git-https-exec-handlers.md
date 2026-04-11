---
status: complete
priority: p3
issue_id: 751
tags:
  - code-review
  - security
  - identity
  - exec-handlers
dependencies:
  - "#515"
  - "#517"
---

# Extend GH_TOKEN identity threading to git HTTPS in exec handlers

## Problem Statement

Commit `859c2dd` (#515) re-injects `GH_TOKEN` into exec handler child processes after `scrub_mika_env_vars()`, fixing the identity collision for `gh` CLI calls inside skills. This closes the parity gap for `gh pr review`, `gh pr create`, etc.

However, the same exec handler subprocess can also invoke `git push` / `git clone` over HTTPS, which authenticates via git credential helpers — not `GH_TOKEN`. After the scrub, no credential helper config is injected, so HTTPS git operations still fall back to host identity (or fail outright).

A `crates/mika-agent/src/skills/git.rs::inject_github_token` helper exists for the skill install path (clone-from-github) but is not applied to general exec handler subprocesses.

**Practical impact:** Low. The current self-dev loop uses SSH for git operations (per `gh auth status` showing `Git operations protocol: ssh`). HTTPS git ops in exec skills are uncommon. But it's a real parity gap if someone writes a skill that does `git push https://github.com/...`.

## Findings

- **Source:** `compound-engineering:review:agent-native-reviewer` review of commit `859c2dd`
- **Existing helper:** `crates/mika-agent/src/skills/git.rs::inject_github_token` — used only for skill install clone path
- **Missing:** Same injection in `execute_exec()` and `spawn_long_running_exec()` for any subprocess that may invoke `git`
- **Equivalent for `gh`:** Just-merged in #515 — sets `GH_TOKEN` env var
- **For `git`:** Would need to set `GIT_ASKPASS` to a helper that returns the token, or write a temporary credential file with `git credential store --file=...`

## Proposed Solutions

### Option A: Set credential helper via env var (preferred)

Set `GIT_ASKPASS=<helper-script>` in exec handler env when `github_token` is `Some`. The helper script (already exists for `mika credential-helper get`) prints the token on stdout. This requires:
- Bundling or generating an askpass shim that calls `mika credential-helper get`
- Or pointing `GIT_ASKPASS` directly at `mika credential-helper get` if its CLI shape allows

**Pros:** No filesystem writes, scoped to subprocess, no leftover state
**Cons:** Adds a runtime dependency on `mika credential-helper get` being on PATH inside skills
**Effort:** Small
**Risk:** Low

### Option B: Wait for a real use case

The current self-dev loop uses SSH. No skill currently does HTTPS git ops. Document the gap and address when it bites.

**Pros:** Zero work, no speculative complexity
**Cons:** Latent gap; first user to hit it gets a confusing failure
**Effort:** None
**Risk:** Low

### Option C: Document the limitation in skills.md

Add a one-line note in `docs/skills.md` warning skill authors that exec handlers can use `gh` (with agent identity) but `git push https://...` falls back to host credentials.

**Pros:** Sets expectations
**Cons:** Doesn't fix the gap, just hides it
**Effort:** Small
**Risk:** None

## Recommended Action

(Triage)

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/executor.rs` (sync + long-running exec paths)
- **Reference:** `crates/mika-agent/src/skills/git.rs::inject_github_token` (existing helper)
- **Related:** ADR-008 (`docs/adr/008-github-identity-separation.md`)

## Acceptance Criteria

- [ ] Decision documented (fix vs defer vs note)
- [ ] If fixing: exec handler subprocesses have working git HTTPS auth as agent identity
- [ ] If fixing: test asserts `git push` works as agent's `MIKA_GITHUB_TOKEN` identity
- [ ] If deferring: limitation documented in `docs/skills.md`

## Work Log

- 2026-04-11: Created from `/ce:review` of commit `859c2dd` (mika#515)
- 2026-04-11: Resolved via Option C (document the limitation). Added a bullet to the "Execution details" list in `docs/skills.md` explaining that exec handler subprocesses receive `MIKA_GITHUB_TOKEN` as `GH_TOKEN` for `gh` CLI but do not receive credential helper injection for `git` over HTTPS — skill authors should use SSH remotes or open an issue for `GIT_ASKPASS` support. Option A (full `GIT_ASKPASS` fix) intentionally deferred: practical impact is low because self-dev uses SSH for git operations, and no current skill needs HTTPS git ops.

## Resources

- Commit: `859c2dd` (fix(skills): inject GH_TOKEN into exec handler child processes)
- Related issues: mika#515, mika#517
- ADR: `docs/adr/008-github-identity-separation.md`
- Existing pattern: `crates/mika-agent/src/skills/git.rs`
