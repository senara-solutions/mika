---
title: A structural CI guard fails open in three places — its parser, its fixture, and its harness
date: 2026-08-29
category: best-practices
module: scripts, skills/bundled/_shared
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - "Writing a CI lint that asserts a deny-by-default invariant over source text"
  - "Writing the anti-vacuity test for such a lint (proving it can fail)"
  - "Adding a live probe to a harness that runs under `set -euo pipefail`"
  - "Reviewing a change that ships a new structural guard"
related_components:
  - testing_framework
  - development_workflow
tags:
  - structural-guard
  - ci-gate
  - fail-open
  - deny-by-default
  - anti-vacuity
  - pipefail
  - sigpipe
  - mutation-testing
---

# A structural CI guard fails open in three places — its parser, its fixture, and its harness

## Context

mika#2039 moved the pilot's GitHub PAT off the bwrap command line. `--setenv GH_TOKEN <value>` put an org-scoped write token into a world-readable `/proc/<pid>/cmdline`; the token now travels on a file descriptor and is materialised as a 0600 file inside the sandbox (`skills/bundled/_shared/dispatch-lib.sh:293-300` for the secret allowlist, `:458-470` for the descriptor and `--ro-bind-data`, `:527` and `:600` where those args reach each bwrap invocation).

Closing the leak was the easy half. The branch also shipped the machinery meant to keep it closed: a name-side CI lint (`scripts/verify-no-secret-in-setenv.sh`), a value-side unit suite against a mocked bwrap (`skills/bundled/_shared/tests/test_sandbox_no_secret_in_argv.sh`), and a live probe on a real sandbox (`scripts/canary-pilot-containment` PART 0, `:147-241`). The first two were wired into CI and `make test` (`.github/workflows/ci.yml:87-90`, `Makefile:123-131`, `:139-144`); the canary is operator-run after a deploy and is referenced by no Makefile target and no workflow.

A nine-persona review (session history) then found that each of the three had a way to go green while the leak was live — and the three failure modes were not variations on one bug. They sat at three different layers, and each was **silent**: no error, no skip notice, no red build.

1. **The parser.** `extract_array` in the lint (`scripts/verify-no-secret-in-setenv.sh:103-110`) is an awk extractor that anchors on `^NAME=(` and exits at the first `^)`. Rule 1 compares what it returns against a literal audited set (`:53-66`, `:166-186`) — genuine deny-by-default, in principle. But deny-by-default only denies what the parser can see. Verified by re-running the lint as it stood in this branch's first guard commit (mika#2039, before the review pass) this session against a copy of the current `dispatch-lib.sh` with one line appended:

   ```
   _PILOT_SANDBOX_ENV_ALLOWLIST+=(NPM_TOKEN ATLASSIAN_API_TOKEN)
   ```

   Output: `verify-no-secret-in-setenv: clean — no secret reaches the sandbox argv.` — exit 0. Two credential-named variables back on the `--setenv` path, the deny-by-default lint reporting a clean audited set, and (per the review) the value suite unaffected because the append changed no code the mock exercised.

2. **The fixture.** The suite's anti-vacuity case proved "this lint rejects the pre-fix shape" by reading the pre-fix file out of history: `git show origin/main:skills/bundled/_shared/dispatch-lib.sh`, with an `else` branch printing `⊘ skipped — origin/main not fetched in this checkout`. Both halves fail. The ref becomes the *corrected* file the moment the branch merges, so on main the two assertions invert and every subsequent PR goes red. And on PR runs it never ran at all: the `check` job's checkout sets no `fetch-depth` (`.github/workflows/ci.yml:56`), so `origin/main` is not present, the `git show` fails, and the case takes the skip branch — printing a notice into a log nobody reads while the suite reports success.

3. **The harness.** `scripts/canary-pilot-containment` runs under `set -euo pipefail` (`:46`). The isolation driver used to exercise the new PART 0 probe during development did not. Under the real flags the probe kills the whole canary on its first poll tick: `pgrep -P "$_canary_bg" bwrap` exits 1 before the sandbox exists, `head -1` would normally mask that, `pipefail` promotes it to the pipeline's status, and `set -e` aborts. Reproduced minimally this session — `bash -c 'set -euo pipefail; x=$(pgrep -P 99999 bwrap 2>/dev/null | head -1); echo reached'` prints nothing and exits 1. The fix is one suffix, `|| _bwrap_pid=""` (`:189`), with the reason written down beside it (`:184-188`).

Five of the nine reviewers (session history) **reproduced** their finding rather than argued it — each built the mutant, ran the guard, and pasted the exit code. That is what separated the real findings from the plausible ones: every reproduced finding survived verification, and the reproduction scripts became the regression tests almost verbatim (`scripts/test-verify-no-secret-in-setenv.sh:172-215`, whose comments say "reproduced during review" three times). `scripts/test-verify-no-secret-in-setenv.sh` is now 25 assertions and the value-side suite 29, all passing, measured this session.

## Guidance

**Treat a structural guard as three artifacts, and ask the fail-open question of each separately.**

**The parser: fail closed on syntax you cannot model.** A narrow extractor is fine — desirable, even. What is not fine is returning a partial answer indistinguishable from a complete one. Add a rule *before* the audit that rejects any form the extractor was not built for, and say so in the extractor's own comment so the coupling is not accidental (`scripts/verify-no-secret-in-setenv.sh:99-102`). Concretely, Rule 0b pins the array to exactly one plain `NAME=(` opener and exactly one total assignment, counting `+=`, a second assignment, and `declare` forms as violations (`:112-126`). Rule 0c does the same for the emit side: exactly one sanctioned dynamic `--setenv` producer, pinned by literal count, and any `--setenv` whose name is not a bare literal — `"$var"`, a backslash line-continuation before the name — is rejected outright (`:128-156`). Two related habits belong here: make an exemption per-occurrence, not file-global (a whole-file `grep -q` for the audited placeholder still passes when a second `--setenv ANTHROPIC_API_KEY` carrying a real key is added beside it — `:205-222`), and scan a comment-stripped view so a guard can explain the pattern it forbids without failing CI on its own prose (`:89-94`).

**The fixture: synthesize the negative case, never fetch it.** An anti-vacuity test that reads the broken state out of git history is coupled to where history is standing. Build the mutant from the live file instead — `cp`, then one targeted edit (`scripts/test-verify-no-secret-in-setenv.sh:73-82`). The property under test is "this shape is rejected", which is a statement about the shape, not about a ref. And a case that cannot run is a **failure, not a skip**: the delimiter test builds a mutant lint with the `(^|_)PAT(_|$)` delimiters removed, and if the mutant cannot be constructed it increments `FAIL` with an explicit "this is a failure, not a skip" message (`:97-106`). While you are there, check that each assertion routes through the rule it names — the old "PATH stays legitimate" case ran the lint on the unmodified file, where `PATH` only ever reaches Rule 1's set-equality check and never touches Rule 2's name pattern at all. It now plants a literal `--setenv PATH` to force it through (`:86-95`).

**The harness: exercise the probe under the flags it will actually run with.** A driver that omits `-e`, or `pipefail`, or `-u`, is testing a different program. This is where the second-order lesson bites, below.

**Reproduce, don't argue.** In review of a guard, a finding stated as reasoning is a hypothesis; a finding stated as `<mutant> → exit 0` is a fact plus a ready-made test. Ask for the mutant. The five reproduced findings on this branch all held; the reproduction cost was minutes and paid for itself twice, once as evidence and once as coverage.

**Second-order: a prose learning did not prevent its own recurrence at n=1.** `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md` records that `printf "%s" "$h" | grep -q "$n"` is a SIGPIPE trap under `pipefail` — `grep -q` exits at the first match, `printf` takes SIGPIPE and exits 141, `pipefail` promotes it, and the assertion reports "absent" for a value that is present. It landed in `b84fdbc8`; the only two commits between it and this branch's merge-base with main are dependabot bumps, so it sat in the tree, unchanged, for the whole of this work. The very next commit on this branch wrote `printf '%s' "$_argv" | grep -qE "$_CANARY_CRED_PATTERN"` into PART 0 of the canary — a security probe whose false-negative branch prints `PASS: no credential-shaped value in the live sandbox argv`. The plan's named reference form for this work, `scripts/verify-egress-no-log.sh:234`, already used the herestring; so does `scripts/check-secrets.sh:126`. The doctrine those files carry in their own headers — "construct the incapacity, don't promise the restraint" (`scripts/verify-egress-no-log.sh:8-11`) — argues that this pattern is now owed a lint rather than another paragraph.

Such a lint would assert, over `scripts/**` and `skills/bundled/**` shell files that set `pipefail`: **no pipeline whose right-hand side short-circuits may have a data producer on its left** — in the checkable form, reject `printf ... | grep -q` and `echo ... | grep -q`, with `<<<` as the sanctioned replacement and a per-line escape hatch requiring a ticket citation, mirroring the `# voice-non-transit: safe #<ticket>` form that `scripts/verify-voice-non-transit.sh:28-33` already enforces structurally. It would not be green today: 9 files still carry the shape, 8 of them under `pipefail`, including 10 occurrences in `skills/bundled/_shared/dispatch-lib.sh` itself (`:768`, `:1046`, `:1128`, `:2032`, `:2199`, `:3337`, `:3347`, `:3642`, `:3858`, `:3900`) — several over exactly the large haystacks (PR bodies, push error output) whose size the prior doc identifies as the risk multiplier. That backlog is the argument for the lint, not against it.

## Why This Matters

Every one of the three failures produces the same observable as a healthy guard: a green check. A guard that can only be wrong loudly is a guard; a guard that can be wrong quietly is a claim. The parser bug is the sharpest case, because the lint's header advertises deny-by-default posture and its message tells a reader that any addition, removal, or rename fails "regardless of how the new name looks" (`scripts/verify-no-secret-in-setenv.sh:23-28`) — the advertisement was true of the rule and false of the file, and only the rule was tested. The confidence the guard bought is exactly what makes the false negative expensive: the next reviewer sees a green deny-by-default lint over the secret channel and stops looking.

The fixture failure is worse than a missing test, because it was scheduled to break the wrong thing at the wrong time: silently absent on every PR run, then loudly red on main for every unrelated PR the day after merge. The harness failure had the same asymmetry in miniature — the probe would abort the canary before any of its parts ran, so an operator running the containment check after a deploy would see a truncated output rather than a failure.

Two prior-session findings sharpen this (session history). The SIGPIPE defect was itself found by an adversarial reviewer, and it retroactively explained a failure the session had already written off: the suite had failed once, then run green four consecutive times (381/381), and the orchestrator logged the single red as "transitoire inexpliqué". **Four consecutive green runs did not disprove a real fail-open.** In the same session, a negative control caught a complacent assertion of exactly the shape described here — deleting one reason-setter left a count assertion green ("21 ≥ 18") while a new per-site assertion went red; the count assertion was removed. The ratified template for this class in this workspace is control-monitor PR#143: read `PIPESTATUS[0]` on the immediately following line, take the verdict from which marker phrase appeared rather than from a count, treat "verdict OK but non-zero exit" as failure — *a gate that disagrees with itself is not a pass* — and carry anti-vacuity assertions that verify the guarded idiom still exists, so the test cannot silently become a no-op.

And the second-order point generalises past bash: this repo's own memory records that prompt-level and prose-level enforcement fails at the substrate. Here the enforcement was prose in `docs/solutions/`, the recurrence interval was one commit, the author had read the doc (the fix cites it, `scripts/verify-no-secret-in-setenv.sh:191-194`), and the recurrence still happened — inside the security probe. n=1 across one commit is not a large sample, but it is the cheapest possible refutation of "we wrote it down, so it will not recur."

## When to Apply

Apply this on any change that adds or edits a guard under `scripts/` or a `test_*.sh` beside it. This repo wires structural guards to CI constantly, and they do not all carry the same exposure. Honestly split:

**Share the parser exposure — a grep/awk model of source that some valid syntax escapes, failing open:**

- `scripts/verify-egress-no-log.sh` — its `production_lines` awk function (`:120-165`) models Rust `#[cfg(test)]` scoping. It recognises `mod NAME;` and `mod NAME {`; for *any other* form after the attribute — a bare `#[cfg(test)] fn`, a `#[cfg(test)] impl` — it calls awk `exit` (`:160`), silently dropping the remainder of the file from the scan. The comment beside it says the case is "worth a manual lint revisit" — a promise, in a file whose header preaches construction over promise. Same class as mika#2039's parser, unfixed. Its `info!` allowlist has a second, milder one: the check reads an 8-line window after the call (`:229-230`), so an `info!` with a non-allowlisted event passes if an allowlisted `event = "…"` token happens to sit within 8 lines below it.
- `scripts/check-loop-select.sh` — an awk brace-depth tracker delimits the `run_loop` body (`:47-95`). It handles double-quoted strings and `//` comments; block comments are explicitly not handled and the header says so (`:44-46`), so a `}` inside a `/* */` inside `run_loop` truncates the scanned range and a `tokio::select!` after it escapes. Note the contrast in its other half: when it cannot find `^async fn run_loop(` it exits 2 rather than 0 (`:34-38`) — that half already fails closed, which is the shape the parser half is missing.

**Do not share it — no source model to escape:**

- `scripts/verify-egress-uniqueness.sh` and `scripts/verify-voice-non-transit.sh` are literal-substring denylists over whole files, plus path allowlists. Their exposure is list completeness (a new upstream host, a new cloud SDK), which is visible in the array itself and documented as the extension point — a known-incomplete list is not a guard that lies about its coverage.
- `scripts/check-byte-slices.sh` is pure `grep -rn` over `crates/` with an opt-in `// safe-byte-slice:` escape (`:16-38`). It never claims exhaustiveness, so a miss is an expected denylist gap rather than a silent audit of a subset.
- `scripts/check-secrets.sh` is a regex denylist (`:22`), but it is *harness*-coupled — diff-scoped against `git merge-base` — and it handles that coupling the way the mika#2039 fixture did not: `-z` NUL-delimited diff output with a comment explaining that C-quoted non-ASCII paths would otherwise be skipped as "not present" and reported clean (`:68-74`), and CI pins `fetch-depth: 0` on the job that runs it (`.github/workflows/ci.yml:210-214`). Read it as the worked example of a git-history-coupled guard done right, and note the contrast: the job running the new mika#2039 guards has no `fetch-depth` at all (`:56`), which is exactly what made the old fixture skip.

Apply the harness rule whenever a probe is added to a script that sets `-e`, `-u`, or `pipefail` — run the probe inside the script, not in a hand-rolled driver.

## Examples

### Parser: audit a subset silently → refuse to audit what you cannot model

Before — as first written on this branch, the only defence was the extractor itself, and it stopped at the first `)`:

```bash
extract_array() {
    local name="$1"
    awk -v n="$name" '
        $0 ~ "^" n "=\\(" { inside = 1; next }
        inside && /^\)/    { inside = 0; exit }
        inside             { print }
    ' "$TARGET" | tr -s ' \t' '\n' | grep -v '^$' || true
}
```

After (`scripts/verify-no-secret-in-setenv.sh:112-126`) — the narrowness is made safe by a rule that runs first:

```bash
# --- Rule 0b: the arrays must appear in exactly the one form we can parse --
for _arr in _PILOT_SANDBOX_ENV_ALLOWLIST _PILOT_SANDBOX_SECRET_ALLOWLIST; do
    _opens=$(grep -cE "^${_arr}=\\(" "$CODE_ONLY" || true)
    _touches=$(grep -cE "^[[:space:]]*(declare[[:space:]]+-[a-zA-Z]+[[:space:]]+)?${_arr}\\+?=" "$CODE_ONLY" || true)
    if [[ "$_opens" -ne 1 || "$_touches" -ne 1 ]]; then
        echo "VIOLATION: $_arr is written in a form this lint cannot audit."
        ...
```

Measured this session on the identical input (`dispatch-lib.sh` + one appended `_PILOT_SANDBOX_ENV_ALLOWLIST+=(NPM_TOKEN ATLASSIAN_API_TOKEN)` line): old lint exit 0, "clean"; new lint exit 1, "written in a form this lint cannot audit — Found 1 plain `NAME=(` opener(s) and 2 total assignment(s)."

### Fixture: fetch the broken state from history → synthesize it by mutation

Before — coupled to `origin/main`, and skipping rather than failing when the ref is absent:

```bash
PREFIX_FIXTURE="$TMPROOT/dispatch-lib.prefix.sh"
if git -C "$REPO_ROOT" show origin/main:skills/bundled/_shared/dispatch-lib.sh \
        > "$PREFIX_FIXTURE" 2>/dev/null; then
    R=$(run_lint "$PREFIX_FIXTURE")
    assert_exit "pre-fix form: exit 1" "1" "$R"
    assert_mentions "pre-fix form: names GH_TOKEN" "GH_TOKEN" "$R"
else
    echo "  ⊘ skipped — origin/main not fetched in this checkout"
fi
```

After (`scripts/test-verify-no-secret-in-setenv.sh:73-82`) — the property, stated without a ref:

```bash
# Synthesized from the live file, NOT read from `origin/main`. That ref is the
# pre-fix file only until this work merges; afterwards it IS the corrected file
# and the assertions below would invert, turning main red on every subsequent
# PR. A fixture built by mutation states the property without depending on
# where history happens to be standing.
F=$(fixture "prefix-gh-token.sh")
perl -0pi -e 's/(_PILOT_SANDBOX_ENV_ALLOWLIST=\(\n)/$1    GH_TOKEN\n/' "$F"
R=$(run_lint "$F")
assert_exit "GH_TOKEN back on the --setenv allowlist: exit 1" "1" "$R"
assert_mentions "pre-fix shape: names GH_TOKEN" "GH_TOKEN" "$R"
```

### Harness: the one-suffix fix, with the reason kept next to it

`scripts/canary-pilot-containment:184-189`:

```bash
        # `|| _bwrap_pid=""` is load-bearing: this script runs under
        # `set -euo pipefail`, and on every poll iteration before the sandbox
        # appears `pgrep` exits 1. `head` would mask it, but `pipefail`
        # promotes it to the pipeline's status and `set -e` would abort the
        # whole canary on the first tick — silently, before any probe ran.
        _bwrap_pid=$(pgrep -P "$_canary_bg" bwrap 2>/dev/null | head -1) || _bwrap_pid=""
```

The same probe also stopped calling an empty `/proc/<pid>/cmdline` read a PASS (`:202-206`), extended its credential pattern to include the `canary-must-not-leak` marker the script stamps on its own decoys (`:159-165` — the four decoys match no vendor prefix, so the probe would otherwise have printed PASS with them sitting in the argv), and switched every match to a herestring (`:214`, `:217`, `:224-225`, `:233`).

## Related

- `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md` — the SIGPIPE-under-`pipefail` mechanics in full, and the harness-hermeticity work that preceded this. This entry is the grounding case for the harness third above; mika#2039 is its second independent instance, which is what moves the pattern from one-off to recurring.
- `docs/solutions/ci-cd/2026-08-27-porting-a-precommit-detector-to-ci-parity-traps.md` — the same "a detector can go green because parity was assumed, not proven" shape, two days earlier, on `check-secrets.sh`.
- `docs/solutions/best-practices/test-fixture-content-coverage-2026-05-02.md` — the fixture-vacuity family from a different angle: fixtures that cover structure but not production-realistic content.
- `docs/solutions/best-practices/structural-guards-vs-doc-comments-2026-06-13.md` — the tier argument (structural over prose). This entry is its sequel: a structural guard can still fail open, so the tier is necessary and not sufficient.
- `docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md` — the founding structural-gate precedent for this repo.

Origin: mika#2039.
