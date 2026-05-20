# Plan: make deploy logs built SHA and warns when local main lags origin/main (mika#1210)

type: fix (deploy hygiene / operator observability)
ticket: mika#1210
date: 2026-05-20
groomed-via: /mika-groom-ticket (/ce:plan → mika-arch first-pass → revisions → mika-arch second-pass)

## Problem

`make deploy` builds from the local clone without surfacing what SHA is being built or whether the local checkout is stale relative to `origin/main`. When `git pull` is skipped or fails silently, the build completes, services restart, `check-ngrok` passes, and the operator believes the new code is live when it isn't. Discovery requires a manual `git log --oneline origin/main` comparison. The exact gap fired twice in the last two days — 2026-05-18 (the originally documented incident, 4 commits behind) and 2026-05-19 (the deploy described in the 2026-05-19 handsoff log, 8 commits behind because `fetch origin main:main` refused while main was checked out, so the local main never advanced). Operator wastes wall-clock hours diagnosing "why isn't the fix live?"

## Phase 0 — Pin (base SHAs and verbatim slices)

Base SHAs at grooming time:
- `mika` HEAD: `498c536a18de83f69216aefc330d321f22277163` (`fix(agent): tighten milestone-close-claim guard to first-person + number cross-ref (#1207) (#1216)`)
- `mika-platform` HEAD: `5393379946754e49e1033ba38b0ec48b40eeee1f` (`docs(logs): mika backlog triage report 2026-05-20`)

All implementation must be applied against these SHAs. Line numbers below refer to file state at these SHAs.

### Pin 1 — `mika/Makefile:4` (`.PHONY` declaration, single-line)

```make
.PHONY: build build-dashboard deploy stop restart install test lint fmt check check-ngrok clean help calibrate-mika-dev calibrate-mika-arch
```

The `.PHONY` list is a single line. AC1's new target `deploy-info` is appended to this line (one token before `clean help`, alphabetical isn't enforced — placement is "next to other deploy-adjacent targets," e.g. between `check-ngrok` and `clean`).

### Pin 2 — `mika/Makefile:45` (current `deploy` line)

```make
deploy: build-dashboard build install restart check-ngrok ## Full deploy: build, install, restart
```

AC3 inserts `deploy-info` as the first prerequisite (before `build-dashboard`). The `## ` help comment must stay byte-identical so `make help` output doesn't change.

### Pin 3 — `mika/Makefile:47-54` (`check-ngrok` recipe — fail-soft warning precedent)

```make
check-ngrok: ## Warn if ngrok is not running (Telegram webhooks need it)
	@if ! curl -sf http://localhost:4040/api/tunnels > /dev/null 2>&1; then \
		echo ""; \
		echo "  ⚠  WARNING: ngrok is not running!"; \
		echo "  Telegram webhooks will not reach the gateway."; \
		echo "  Start ngrok: ngrok http 8080"; \
		echo ""; \
	fi
```

Shape constraint for AC2/AC6: the new `deploy-info` recipe mirrors `check-ngrok`'s pattern — `@`-prefixed multiline `if` block, no `&&`/`||` chains relying on shell short-circuit, no `; exit 0` workaround. The conditional itself controls the exit code: a Bourne `if … fi` without `set -e` ends at zero regardless of which branch fired, so the recipe's exit code is the exit code of the final `fi`. Implementer must NOT introduce `set -e` inside the recipe body.

### Pin 4 — `mika/Makefile:64-66` (`test` target — AC8 wiring precedent)

```make
test: ## Run all tests
	cargo test
	@bash scripts/test-dispatch-symmetry.sh
```

The `test` target EXISTS and already delegates to a bash script with the `@bash scripts/<name>.sh` shape. AC8's wiring is feasible as a one-line append: a third recipe line `@bash scripts/deploy-info-test.sh`. No new target creation needed.

### Pin 5 — `mika-platform/Makefile:1-2` (`.PHONY` declaration, multi-line) and `:42` (`deploy` alias)

```make
.PHONY: build install restart deploy check-ngrok \
       build-mika install-mika install-claude-pilot install-skills restart-mika
```

```make
deploy: build install restart check-ngrok ## Build, install, restart, and verify ngrok
```

The meta-repo `.PHONY` is multi-line with backslash continuation; AC4's new `deploy-info` token is appended to the continued second line (after `restart-mika`). The `deploy` alias at `:42` (NOT `:46` — earlier draft had wrong line number) is a pure-prereq chain target; AC4 inserts `deploy-info` as the first prerequisite, recipe body stays absent.

## Spec deviation from issue body

The issue body's "Proposed Solution" inlines the SHA print + fetch + warn block directly into the `deploy` recipe (single-target, recipe-body shape). This plan promotes that block to a separate `deploy-info` prerequisite target on both Makefiles. The deviation is intentional and structurally justified:

1. **DRY across both Makefiles.** The meta-repo `make deploy` and the `mika/` direct `make -C mika deploy` are two operator-facing invocation paths that hit the same failure mode. An inline recipe in `mika/Makefile`'s `deploy` only catches the direct path; the meta-repo path needs the same recipe duplicated, or — cleaner — needs to delegate to a reusable target. `deploy-info` is that reusable target.
2. **Standalone-queryable.** An operator who wants to ask "what would deploy build right now, and am I current?" without running the full deploy pipeline can run `make -C mika deploy-info` and get the answer in <2s. The inline shape doesn't expose that query path.
3. **Help-listable.** A separate target with a `## ` help comment shows up in `make help`, surfacing the freshness check as a first-class operator verb. Inline recipe bodies aren't help-listable.

This deviation will be recorded in the PR description, and the issue body's "Proposed Solution" section will be updated at PR time with an edit notice citing this plan. Per the issue-as-versioned-contract doctrine (audit trail = body edit + edit-notice comment + closure annotation in plan), the deviation must be visible — not assumed.

## Context

- **Two `Makefile`s have a `deploy` target.** `mika/Makefile:45` (Pin 2) is `deploy: build-dashboard build install restart check-ngrok` — a pure chain target with no recipe body. `mika-platform/Makefile:42` (Pin 5) is the convenience alias the operator typically runs; it delegates to `$(MAKE) -C mika build-dashboard` and `$(MAKE) -C mika build` for the build step, NOT to `make -C mika deploy`. So a prelude attached only to `mika/Makefile`'s `deploy` target catches the literal repro path from the ticket but **not** the operator's typical invocation. The 2026-05-19 incident actually used the meta-repo path (`make deploy` from `/data/workspace/mika-platform/`). Both paths need the warning to make the fix felt.

- **`git fetch origin main:main` is refused when main is checked out.** Hit while creating this worktree from main moments ago, hit on the 2026-05-19 deploy, hit in every workflow that calls that exact refspec form. The fix is to fetch the remote ref WITHOUT updating the local branch — `git fetch -q origin main` (no `:main` refspec) lands the update on `FETCH_HEAD` and `refs/remotes/origin/main` without touching the local `main` ref. This is what the ticket's proposed solution already uses, and it works regardless of which branch is checked out.

- **Fail-soft on network errors.** If origin is unreachable (no network, GitHub outage), `git fetch` exits non-zero. The prelude must not block deploy in that case — it must print "couldn't reach origin" and continue. The ticket's proposed solution swallows fetch errors with `2>/dev/null && ...`; the implementation here makes the fail-soft path explicit (an "origin unreachable" notice) so the operator can distinguish "checked, up to date" from "didn't check."

- **Warn, do not fail.** The ticket's expected behavior lists "warn (or fail)". Hard fail blocks legitimate workflows (deploying a feature branch ahead of main, deploying on a flight with no network, hot-fix on a divergent branch). Warn-only is the right default; the operator chooses whether to abort. Same posture as `check-ngrok`, which warns and continues.

- **HEAD-vs-origin/main check is the right shape.** `git rev-list --count HEAD..origin/main` counts commits in `origin/main` not yet in `HEAD`'s history. If HEAD is main and main is N commits behind, the count is N — exactly what the operator wants to see. If HEAD is a fresh feature branch (main + 1 local commit), origin/main has 0 commits not yet in HEAD's history (assuming branched from current main), so the count is 0. If main moved while operator was on the branch, the count surfaces that drift — which is also a valid signal. The check is conservative-and-informational, not gating.

- **Branch identity in the warning.** The current branch matters — "you are on `main`, 4 commits behind origin/main" carries different weight than "you are on `fix/some-branch`, 4 commits behind origin/main". Include `$(git rev-parse --abbrev-ref HEAD)` in the warning text so the operator sees both signals at once.

- **Built-SHA scope.** The SHA printed is `git rev-parse --short HEAD` from the mika checkout — that is what `cargo build` compiles. Note: this is "what the build inputs are at recipe start," not "what landed in the binary after a successful build" (uncommitted changes still build into the binary). Surfacing uncommitted-change status is left for follow-up; the ticket's repro path is fully-committed-but-stale, and adding a worktree-dirty check expands the surface area without addressing the documented incident class.

## Acceptance criteria

- **AC1 — New `deploy-info` target in `mika/Makefile`** that, when invoked standalone (`make -C mika deploy-info`), prints to stdout:
  ```
  Building from: <abbrev-ref> @ <short-sha> (<commit-subject>)
  ```
  using `git rev-parse --abbrev-ref HEAD`, `git rev-parse --short HEAD`, `git log -1 --pretty=format:'%s'`. The target must be `.PHONY` (added to the existing `.PHONY` declaration on Pin 1 — `mika/Makefile:4`) and have a `## ` help comment so it appears in `make help` (the existing help target uses a grep-on-help-comment pattern).

- **AC2 — `deploy-info` performs an origin/main divergence check** with three observable outcomes:
  1. **Up-to-date:** `git fetch -q origin main` succeeds AND `git rev-list --count HEAD..origin/main` returns `0`. Print `origin/main: up to date`.
  2. **Behind:** fetch succeeds AND count is `N > 0`. Print `WARNING: HEAD is N commits behind origin/main. Run 'git pull --ff-only' if you intended to deploy origin/main.`
  3. **Unreachable:** fetch fails (exit non-zero). Print `NOTE: could not reach origin (network/auth) — skipping freshness check.`

  None of the three outcomes exit the target non-zero — the target always succeeds, the operator reads the line. (Behind/unreachable are advisory signals, not deploy gates; AC3 anchors that posture.)

- **AC3 — `deploy` target in `mika/Makefile` depends on `deploy-info` as its first prerequisite.** Rewrite Pin 2 (`mika/Makefile:45`) from:
  ```
  deploy: build-dashboard build install restart check-ngrok ## Full deploy: build, install, restart
  ```
  to:
  ```
  deploy: deploy-info build-dashboard build install restart check-ngrok ## Full deploy: build, install, restart
  ```
  This guarantees the SHA line and divergence check run BEFORE any build/install/restart step, so the operator sees the freshness signal at the top of the deploy output. The `## ` help comment on `deploy` stays byte-identical.

- **AC4 — `mika-platform/Makefile`'s `deploy` target also surfaces the mika freshness signal.** Add a `deploy-info` prerequisite (matching the convention) that delegates to `$(MAKE) -C mika deploy-info`. Per Pin 5: the meta-repo `.PHONY` list grows by one entry on the second continued line (after `restart-mika`). Existing line `:42` becomes `deploy: deploy-info build install restart check-ngrok ## Build, install, restart, and verify ngrok`. The new `deploy-info` recipe is one line: `$(MAKE) -C mika deploy-info`. This covers the operator's typical invocation path (the 2026-05-19 incident).

- **AC5 — Recipes are quiet by default.** Use the `@` recipe prefix on every line of the new targets so make does not echo the commands themselves (only the resulting output lines are visible). Mirrors the `check-ngrok` precedent (Pin 3 — `mika/Makefile:47-54`).

- **AC6 — Exit-zero discipline inside the recipe.** Mirrors the `check-ngrok` precedent (Pin 3): no `&&`/`||` chains that depend on shell short-circuit to swallow failure, no `set -e` introduced inside the recipe body. A Bourne `if … fi` block ends at exit zero on every branch (the conditional ITSELF carries the exit code, not the commands inside it), so the recipe is exit-safe by construction. Pattern (illustrative — implementation may differ in surface syntax as long as the if/fi-block-as-exit-control shape holds):
  ```make
  deploy-info: ## Print built SHA and warn if local HEAD is behind origin/main
  	@echo "Building from: $$(git rev-parse --abbrev-ref HEAD) @ $$(git rev-parse --short HEAD) ($$(git log -1 --pretty=format:'%s'))"
  	@if git fetch -q origin main 2>/dev/null; then \
  	  AHEAD=$$(git rev-list --count HEAD..origin/main 2>/dev/null || echo 0); \
  	  if [ "$$AHEAD" -gt 0 ]; then \
  	    echo "WARNING: HEAD is $$AHEAD commits behind origin/main. Run 'git pull --ff-only' if you intended to deploy origin/main."; \
  	  else \
  	    echo "origin/main: up to date"; \
  	  fi; \
  	else \
  	  echo "NOTE: could not reach origin (network/auth) — skipping freshness check."; \
  	fi
  ```

- **AC7 — Existing targets unaffected.** No change to `build`, `build-dashboard`, `install`, `restart`, `check-ngrok`, `calibrate-*`, `test`, `lint`, `fmt`, `check`. The diff is additive: one new target on each Makefile, one prerequisite added to each `deploy` line. No reordering of existing prerequisites.

- **AC8 — Verification script.** Add `scripts/deploy-info-test.sh` (executable, bash, follows the style of the existing `scripts/verify-pipeline-test.sh` / `scripts/check-byte-slices.sh`) that exercises the three AC2 paths via a disposable git fixture: (a) up-to-date — fresh clone of an in-test bare repo, `deploy-info` says `origin/main: up to date`; (b) behind — add commit to origin's main without pulling locally, `deploy-info` warns with the correct count and "behind origin/main" string; (c) unreachable — point `origin` URL to a nonexistent path, `deploy-info` prints the "could not reach origin" note and exits zero. Wire it into `make test` (Pin 4 — `mika/Makefile:64-66`) by appending one line `@bash scripts/deploy-info-test.sh` after the existing `@bash scripts/test-dispatch-symmetry.sh` invocation; the `test` target already exists with the `@bash …` shape, so wiring is a one-line append. The fixture uses `mktemp -d`, sets up bare + worktree clones, runs the `deploy-info` recipe via `make -f <fixture-Makefile> deploy-info`, and asserts on grep matches; on exit (success or failure) it cleans up the temp dir. The fixture Makefile is a small file checked into `scripts/fixtures/deploy-info-Makefile` containing only the `deploy-info` target body verbatim — keeping the recipe text out of the test shell script preserves a single source of truth for the recipe and limits drift to one place. The fixture target must be kept byte-identical to the production recipe; the test script asserts this with a diff before running the behavior cases (so any drift fails the test immediately, not at deploy time).

- **AC9 — Manual verification record.** Run `make -C mika deploy-info` in the current worktree (HEAD at `fix/1210/...` branched from main). Capture the output and paste it into the PR description under a "Verification" heading. Also: confirm in PR description that `make help` (mika repo) lists the new target with the help comment.

- **AC10 — Doc note in `mika/CLAUDE.md`.** Under the existing Commands list, change the `make deploy` bullet from:
  > `make deploy` — Full deploy: build dashboard + release binaries with telemetry, install to `~/.local/bin/`, restart services

  to:
  > `make deploy` — Full deploy: build dashboard + release binaries with telemetry, install to `~/.local/bin/`, restart services. Prints the built SHA and warns when local HEAD is behind `origin/main`.

  Mirror the same one-sentence note in `/data/workspace/mika-platform/CLAUDE.md` under "Local Dev Environment" where `make deploy` is described.

## Out of scope (deliberate)

- **Hard fail on divergence.** Warn-only matches the ticket's stated preference and avoids blocking legitimate divergent-branch deploys.
- **Uncommitted/staged-changes warning.** Different failure shape (operator usually knows about local edits); separate concern from the stale-clone class this ticket addresses.
- **Per-sub-repo SHA reporting for claude-pilot-py, mika-skills, mika-cloud.** Adjacent deploy-hygiene concerns but separate failure classes (each has its own freshness semantics — editable install vs copy-based skill update vs Helm chart deploy). File-it-when-needed, not part of this ticket. The PR description should explicitly enumerate this and the previous bullet as named follow-ups so they don't get lost.
- **Fetching all refs (e.g., `git fetch --all`).** The check is specifically against `origin/main` — the deploy source-of-truth. Fetching all refs is slower and not what the operator's failure mode needs.
- **Auto-pull-on-stale.** Would shadow the operator's intent (e.g., operator is on a hot-fix branch on purpose). The warning makes the choice explicit.

## Verification plan

1. **Standalone invocation:** `make -C mika deploy-info` from a fresh main checkout → prints SHA line + `origin/main: up to date`.
2. **Stale local main:** simulated by `git reset --hard HEAD~3` on a disposable worktree, then `make -C mika deploy-info` → prints SHA line + `WARNING: HEAD is 3 commits behind origin/main. Run 'git pull --ff-only' ...`.
3. **No network:** `make -C mika deploy-info` after `iptables`-blocking github.com (or simply pointing origin at an unreachable URL via `GIT_CONFIG_PARAMETERS`) → prints SHA line + `NOTE: could not reach origin ...`.
4. **Deploy path integration:** dry-run `make -C mika deploy` (or scoped to `make -C mika deploy-info build` to skip the install/restart) → confirms `deploy-info` runs first, output is the first lines in the deploy log.
5. **Meta-repo invocation:** `make deploy` from `/data/workspace/mika-platform/` → confirms the freshness signal appears (via the delegated `make -C mika deploy-info`).
6. **Test script:** `bash scripts/deploy-info-test.sh` (or via `make test`) passes locally.

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| `git fetch` adds latency to every deploy (~0.5–2s) | Acceptable cost for the safety signal. Single network call. Operator can `make build` (no prelude) to skip when iterating. |
| Operator on detached HEAD (`abbrev-ref HEAD` returns `HEAD`) | Output still shows the SHA — informative even without branch name. No special-case needed. |
| Operator deploys from a worktree whose `origin` points to a fork (not senara-solutions/mika) | The divergence check compares against `origin/main` regardless. If the fork is behind upstream, that's a different problem the operator owns. Acceptable. |
| The `fetch -q origin main` fails on shallow clones or proxy-only environments | Falls through the AC2.3 unreachable branch — operator sees the note, deploy proceeds. Fail-soft, correct. |
| Trailing exit code from the recipe is non-zero, breaking deploy | AC6 mandates the recipe pattern; AC8's test fixture verifies exit-zero on all three paths. |

## Why this lives in `Makefile`, not a separate script

`Makefile` is the operator's entry point for deploy. A separate `scripts/check-deploy-freshness.sh` would still need to be wired into deploy as a prerequisite, doubling surface area for zero portability gain. Make handles the prerequisite chain cleanly; the recipe is small enough that it stays readable inline. The reusable shape is "depend on `deploy-info` as a prereq," which is exactly what AC3 and AC4 do.

## Related

- mika#1197 — the dispatch-loss fix whose 2026-05-18 deploy first exposed the gap (operator built stale source, believed fix was live, wasted hours).
- mika#1201 — adjacent deploy-hygiene class (pyyaml wipe on `uv tool install`), recently closed.
- 2026-05-19 handsoff log entry: "Decisions in flight — mika#1210 deploy SHA verification still pending" + "Today's deploy hit the exact gap (stale local main due to checked-out branch)."
- `docs/solutions/runtime-errors/uv-tool-install-force-doesnt-reinstall-deps-2026-05-19.md` — sibling deploy-hygiene compound, same class.
