---
module: skills/bundled/_shared/test-dispatch-lib, Makefile, .github/workflows/ci.yml
tags: [bash, testing, sigpipe, pipefail, hermeticity, ci-gate, git-config, dispatch-lib, autonomous-loop]
problem_type: test_failure
category: test-failures
date: 2026-08-29
ticket: mika#1772
applies_when:
  - Promoting a bash test suite that has only ever been run by hand into `make test` or CI
  - Writing or reviewing `assert_contains` / `assert_not_contains` helpers in bash
  - Chasing a bash test failure that does not reproduce on re-run
  - Adding fixtures that `git init` and commit into temp directories
root_cause: test_isolation
resolution_type: test_fix
---

# A never-run bash suite is coupled to the machine that never ran it

## Problem

`skills/bundled/_shared/test-dispatch-lib.sh` held 346 assertions over the autonomous loop's dispatch substrate and was invoked by nothing — not the `Makefile`, not `ci.yml`, only its own header comment. Wiring it into CI (mika#1772) surfaced three defects that had been invisible for as long as it ran only on one developer's machine, by hand, on a green tree.

Two of them would have produced red builds on unrelated PRs. The third made a passing assertion prove nothing.

## Root cause 1 — `printf | grep -q` is a SIGPIPE trap under `pipefail`

Both assert helpers were written as:

```bash
if printf '%s' "$haystack" | grep -qF "$needle"; then
```

`grep -q` exits at the **first** match and closes the pipe. `printf`, still writing the rest of the haystack, takes SIGPIPE and exits 141. The file runs under `set -euo pipefail`, so `pipefail` promotes 141 to the pipeline's status — and the assertion reports a failure for a string that is present.

The failure is probabilistic in the race between grep exiting and printf finishing, so it scales with haystack size and with how early the needle sits. Observed once here on a 32 KB haystack whose needle was at line 78 of 489, then not reproduced across four consecutive runs. A non-reproducible red is the most expensive kind of test failure to chase, and this one was about to become a merge gate.

```bash
# Fix: no pipeline, no SIGPIPE.
if grep -qF -- "$needle" <<<"$haystack"; then
```

The `--` matters independently: a needle beginning with `-` would otherwise be read as an option.

## Root cause 2 — fixtures inherit the developer's global git config

The suite `git init`s temp directories and commits into them. Both behaviours it depends on came from one machine's `~/.gitconfig`:

| Host setting | What happens without it | Measured |
|---|---|---|
| `commit.gpgsign` unset or false | every fixture commit tries to sign | aborts at assertion 247/381, `exit 128`, two temp dirs leaked |
| `init.defaultBranch=main` | `git init` creates `master`; fixtures reference `main` | 7 rebase fixtures fail at `rc=128` |

Nulling the config is the wrong fix. `GIT_CONFIG_GLOBAL=/dev/null` also strips the committer identity several fixtures rely on — measured: 8 failures across the mika#1341 / #1364 / #1407 / #1414 fixtures. Pin only what is needed, additively:

```bash
export GIT_CONFIG_COUNT=5
export GIT_CONFIG_KEY_0=commit.gpgsign;     export GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=tag.gpgsign;        export GIT_CONFIG_VALUE_1=false
export GIT_CONFIG_KEY_2=init.defaultBranch; export GIT_CONFIG_VALUE_2=main
export GIT_CONFIG_KEY_3=user.name;          export GIT_CONFIG_VALUE_3=dispatch-lib test suite
export GIT_CONFIG_KEY_4=user.email;         export GIT_CONFIG_VALUE_4=test@localhost
```

These override the global file rather than replacing it — verified directly: under a global `commit.gpgsign = true`, `git config --get commit.gpgsign` returns `false` with the override in place, and a real commit succeeds.

A third coupling was in the code under test, not the suite: `_post_flight_recovery` hardcoded `/var/log/claude-pilot/${LOG_ID}.log`, a directory holding 4230 real files on the developer machine, so no probe could isolate it. Parameterising it as `${PILOT_LOG_DIR:-/var/log/claude-pilot}` is what made the path testable at all.

## Root cause 3 — assertions that cannot fail

Two shapes, both of which had been passing:

**A needle that can never match.** `declare -f` prints bash's *deparsed* source, not the file text. A needle containing `2>/dev/null` never matches, because the deparser renders it `2> /dev/null`. The assertion was green from the day it was written.

**A totals comparison instead of a per-site check.** A coverage invariant written as `reason_setters >= return_1_sites` measured 22 >= 18 — four units of slack. Deleting one reason setter still passed, at 21 >= 18.

## The check that separated them

Delete the thing the test protects; confirm the test goes red.

Running that here produced the decisive evidence in one command: with one reason setter removed, the totals assertion still reported `✓ (21 >= 18)` while a per-site pairing check reported `✗`. Same tree, same run, one green and one red — which is what "this assertion proves nothing" looks like when you measure it instead of arguing about it.

## Verification

The suite now passes 397/397 under three git-config environments: the developer's normal config, a hostile one setting `commit.gpgsign = true`, and an empty one. Running it under all three is the cheap standing check that the hermeticity has not regressed:

```bash
TMP=$(mktemp -d)
printf '[commit]\n\tgpgsign = true\n' > "$TMP/hostile"; : > "$TMP/empty"
bash skills/bundled/_shared/test-dispatch-lib.sh
GIT_CONFIG_GLOBAL="$TMP/hostile" bash skills/bundled/_shared/test-dispatch-lib.sh
GIT_CONFIG_GLOBAL="$TMP/empty"   bash skills/bundled/_shared/test-dispatch-lib.sh
```

## Prevention

Before promoting any test suite to a merge gate, run it somewhere it has never run. The three defects above were all latent for months and all surfaced within one hour of the suite being pointed at a second environment. The cost of finding them after the gate is live is a red build on someone else's unrelated PR, which is where trust in a gate goes to die.
