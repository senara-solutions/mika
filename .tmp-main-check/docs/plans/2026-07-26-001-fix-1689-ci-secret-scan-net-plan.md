# Plan — fix(ci): CI secret-scan net for rescue-path (--no-verify) draft PRs

**Ticket:** mika#1689
**Type:** fix (defense-in-depth, bug — bounded residual risk, private repo, draft-gated)
**Branch:** `fix/1689/ci-rescue-path-no-verify-mika-1685`
**Target files:**
- `scripts/check-secrets.sh` (new — shared single-source secret + large-file scanner)
- `.github/workflows/ci.yml` (new `secret-scan` job)
- `lefthook.yml` (refactor `no-secrets` / `no-large-files` to call the shared script — DRY coherence)

---

## Context

mika#1685 added `git commit --no-verify` to the three post-flight rescue commit sites in
`skills/bundled/_shared/dispatch-lib.sh` so a leftover clippy nit could no longer wedge the salvage
path. That fix is correct and shipped.

**Side effect:** `--no-verify` is all-or-nothing — it skips the *entire* lefthook `pre-commit` block,
not just lint. `lefthook.yml` `pre-commit` runs lint/typecheck (`rust-fmt`, `rust-clippy`,
`dashboard-lint`, `dashboard-typecheck`, `ui-typecheck`, `toml-syntax` — the intended skip) **plus two
security gates that are NOT lint**:

- `no-secrets` — regex scan for `sk-ant-api*` / `sk-ant-oat*` / `AKIA*` / private-key PEM headers
  (`lefthook.yml:40-47`).
- `no-large-files` — 1 MB per-file cap (`lefthook.yml:49-57`).

On the rescue path `--no-verify` also drops these two. The rescue path **pushes the branch to origin**,
so a secret in pilot-written code would reach the remote draft-PR branch with **no automated net** —
`.github/workflows/ci.yml` runs `cargo fmt --check` + `cargo clippy` + tests + `cargo audit`/`deny`,
but **no secret-scan and no large-file job** (verified: `grep -rinE 'secret|gitleaks|trufflehog' .github/workflows/`
returns only `secrets.GITHUB_TOKEN` refs, no scanner).

**Root cause the fix must close:** the `no-secrets` regex lives in exactly one place — the pre-commit
hook — which `--no-verify` bypasses. A layer the rescue path *cannot* bypass (CI) must carry the same net.

**Boundedness (why this is a bug, not a P0 loop-blocker):** rescue output is a **draft** PR
(operator-gated, never auto-merged); the repo is **private**; secret-prone paths
(`.claude/*.local.*`, `claude-pilot.json`, `.claude/commands/`) are already excluded from rescue
staging (mika#1288/#1419 pathspec); engine-level scrubbing (`secret_scrubber::scrub_secrets()`,
`SecretString`) covers the DB/tool-call layer. Net: low-to-moderate residual risk on a net that used
to run on rescue commits and now never does.

---

## Requirements

- **R1** — CI runs a secret scan on **every PR** (including `wip(` / `wip-rescue` draft PRs), using the
  **same** detection regex as `lefthook.yml`'s `no-secrets` gate — no drift between the two nets.
- **R2** — CI runs a large-file scan (1 MB cap) with the **same** threshold and semantics as
  `lefthook.yml`'s `no-large-files` gate.
- **R3** — The secret regex and the 1 MB threshold live in exactly **one** source of truth. Both the
  CI job and the pre-commit hook consume it, so a future edit to the pattern cannot leave one net stale.
  This directly closes the root cause (regex-in-one-bypassable-place).
- **R4** — CI scan targets the PR's **changed files** (added/copied/modified vs the merge-base with
  `origin/main`), mirroring the pre-commit **staged-files** semantics. This is the net that *would have*
  run at commit time — it must not newly fail on pre-existing large binaries or fixtures already in the
  tree (avoids a false-positive wall on first rollout).
- **R5** — Detection semantics match lefthook exactly, including the existing allowlist excludes
  (`TEST_RSA_PEM`, `secret_scrubber`) and the file-glob scope for secrets
  (`*.{rs,ts,tsx,js,jsx,toml,json,sh,env}`, excluding `lefthook*.yml`). Large-file scan applies to all
  changed files (no extension filter — matches lefthook).
- **R6** — On a hit, the CI job fails (`exit 1`) with an actionable message naming the file (and the
  offending pattern class for secrets); on a clean/empty change set it exits 0.
- **R7** — The new CI job follows the repo's established lint-job pattern
  (`byte-slice-lint` / `loop-select-lint`: `ubuntu-22.04`, `actions/checkout` at the pinned SHA, then
  `bash scripts/check-*.sh`). No new third-party action is introduced (keeps the "all actions pinned to
  commit SHAs" invariant trivially satisfied and avoids a gitleaks/trufflehog supply-chain surface).

---

## Design decisions

**D1 — Shared script `scripts/check-secrets.sh` is the single source of truth (R3).**
Rather than inline the regex a second time in `ci.yml` (which would re-create the drift risk the ticket
flags), extract the `no-secrets` regex + the 1 MB `no-large-files` check into one script. Both lefthook
and CI call it. This is the DRY coherence that structurally prevents the two nets from diverging.

**D2 — Two input modes, one scanner.**
- **Explicit-file mode** (`scripts/check-secrets.sh <file>...`): scans exactly the passed files —
  consumed by lefthook via `{staged_files}`.
- **Changed-set mode** (`scripts/check-secrets.sh --changed <base-ref>`): computes
  `git diff --name-only --diff-filter=ACM $(git merge-base <base-ref> HEAD)..HEAD` and scans those —
  consumed by CI with base `origin/main`.
- **Empty file set → exit 0** (a PR touching no files, or only deletions, is clean).

**D3 — CI scans the diff, not the full tree (R4).**
Diff-scoping mirrors the pre-commit gate's staged-files meaning precisely and avoids a first-rollout
false-positive wall on pre-existing large binaries/fixtures. The secret regex is precise and the tree
has been kept clean by the per-commit hook historically, so full-tree scanning buys little; a
whole-tree secret sweep is explicitly **out of scope** (deferrable follow-up if a stronger posture is
ever wanted). Documented here so a reviewer doesn't read diff-scoping as an oversight.

**D4 — Dedicated `secret-scan` job, not a step folded into `security` or `check` (R7).**
A standalone `ubuntu-22.04` job matches `byte-slice-lint`/`loop-select-lint`, runs fast (checkout +
bash, no toolchain install), reports as its own required check, and keeps the Rust-heavy `check`/`security`
jobs untouched. Needs `fetch-depth: 0` on checkout so `git merge-base origin/main HEAD` resolves —
same precedent as the `pipeline-artifacts` job (`verify-pipeline.sh origin/main`).

**D5 — No new GitHub Action dependency.**
Reusing the in-repo regex via a bash script (vs adopting gitleaks/trufflehog) keeps the change small,
avoids a new pinned-SHA action to vet, and guarantees identical detection to the pre-commit net. A
richer scanner is a separate decision, not this defense-in-depth patch.

**D6 — Regex + threshold copied verbatim from `lefthook.yml`.**
Secret regex: `(sk-ant-api[0-9]+-[A-Za-z0-9_-]{20,}|sk-ant-oat[0-9]+-[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|-----BEGIN (RSA |EC )?PRIVATE KEY)`.
Excludes: lines matching `TEST_RSA_PEM` or `secret_scrubber`; secret glob excludes `lefthook*.yml`.
Large-file threshold: `1048576` bytes. After extraction, lefthook consumes the script so these values
have a single home.

---

## Fire-Disposition

This plan ships **detector-class deliverables** — a secret scanner and a 1 MB large-file scanner
(`scripts/check-secrets.sh`) wired into a new CI `secret-scan` job. Per the Fire-Disposition Gate
(mika#1574), the disposition when a detector fires must be declared against the canonical three-option
schema: **(a) named allowlist exception**, **(b) land-disabled**, **(c) halt-and-surface**. The two
detectors have deliberately *different* dispositions because their false-positive economics differ:

**Rollout-firing is structurally prevented, not merely mitigated.** Both detectors are diff-scoped
(D3/R4): they scan only the PR's changed set (`--diff-filter=ACM` vs merge-base with `origin/main`),
never the pre-existing tree. A secret or oversized blob already committed to `main` therefore *cannot*
fire on an unrelated PR — the class of false positive the gate is most concerned about (detector firing
on legacy data) is closed by construction. The dispositions below govern the residual case: a detector
firing on a file the PR itself adds/modifies.

- **Secret detector → (c) halt-and-surface.** A secret in a *changed* file is the exact event this net
  exists to catch; failing the CI job (`exit 1`, R6) is the intended outcome, not a false positive.
  There is **no content allowlist for real secrets** — the only excludes are the two lefthook-parity
  pattern excludes (`TEST_RSA_PEM`, `secret_scrubber`, D6/R5), which suppress *known test/scrubber
  fixtures*, not live credentials. If a genuine secret is flagged, the author rotates the credential and
  removes it; the gate does not offer a bypass. This is a `halt-and-surface` posture: block the PR,
  surface the offending `file:line` and pattern class, require human remediation.

- **Large-file detector → (a) named allowlist exception.** Unlike secrets, a legitimately-large file
  (e.g. a test fixture, a golden binary) is a real, occasionally-valid need. The disposition is an
  **explicit, named allowlist**, not a blanket bypass:
  - The script carries a `LARGE_FILE_ALLOWLIST` array, **empty at rollout** (diff-scoping means no
    existing fixture forces an entry on the first PR through the gate — see D3).
  - When a PR legitimately needs a file over the 1 MB cap, the author adds that file's exact path to
    the array as a **named** entry with an inline comment stating *why* it is exempt and a tracker
    reference. A path not in the array still halts (`exit 1`). This keeps every exception auditable in
    one reviewed place rather than via an ad-hoc `--no-verify`-style escape.
  - **Follow-up tracker:** no allowlist entry exists or is required at rollout, so no tracker is filed
    now (filing a placeholder issue for a not-yet-needed exception would violate the filing-discipline
    invariant). The **first** PR that adds an allowlist entry MUST accompany it with a one-line
    follow-up issue naming the file and the reason, referenced in the inline comment — the exception is
    never silent. The separately-deferred whole-tree secret sweep (D3) remains the standing posture
    follow-up if a stronger net is ever wanted.

**Why not land-disabled (b):** neither detector is speculative or high-churn — both replicate a net that
already ran per-commit via lefthook and was proven stable there. Landing them disabled would recreate
the exact gap mika#1689 closes (a net that exists but does not run on the rescue path). They land
**active**.

---

## Implementation steps

1. **Create `scripts/check-secrets.sh`** (`chmod +x`, `#!/usr/bin/env bash`, `set -euo pipefail`),
   patterned on `scripts/check-byte-slices.sh`:
   - Parse args: `--changed <base>` → build changed-set via `git merge-base` + `git diff --name-only
     --diff-filter=ACM`; otherwise treat all args as an explicit file list. No args and no `--changed`
     → exit 0 (nothing to scan) with a note.
   - **Secret check:** filter the file set to the secret glob (`.rs .ts .tsx .js .jsx .toml .json .sh
     .env`), drop `lefthook*.yml`; `grep -nE '<regex>'` over the survivors, pipe through
     `grep -v 'TEST_RSA_PEM' | grep -v 'secret_scrubber'`; any surviving match → print
     `ERROR: potential secret in <file>:<line>` and set a violation flag.
   - **Large-file check:** declare a `LARGE_FILE_ALLOWLIST` array near the top of the script, **empty at
     rollout** with a header comment documenting the named-exception contract (Fire-Disposition (a): each
     entry is an exact path + inline `# why + tracker` comment). For every file in the set, skip it if
     its path is a member of `LARGE_FILE_ALLOWLIST`; otherwise `wc -c` and, if `> 1048576`, print
     `ERROR: <file> is <N>KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST with a named reason if legitimate)`
     and set the flag. The allowlist is consulted identically in explicit-file and `--changed` modes, so
     lefthook and CI honor the same named exceptions (single-source, R3/D1).
   - Skip non-existent paths (deleted files can appear in a raw diff even with `ACM` guarding — belt &
     suspenders). Exit 1 if any violation, else 0. Print a one-line "clean" summary on success.

2. **Add the `secret-scan` job to `.github/workflows/ci.yml`** (after `loop-select-lint`):
   ```yaml
   secret-scan:
     name: Secret Scan
     runs-on: ubuntu-22.04
     steps:
       - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
         with:
           fetch-depth: 0
       - name: Scan changed files for secrets and oversized blobs (mika#1689)
         run: bash scripts/check-secrets.sh --changed origin/main
   ```
   (Use the exact pinned checkout SHA already used elsewhere in the file.)

3. **Refactor `lefthook.yml` to consume the script (R3/D1):**
   - `no-secrets.run` → `bash scripts/check-secrets.sh {staged_files}` (keep the `glob` +
     `exclude: "lefthook*.yml"` so lefthook still pre-filters by extension; the script's own glob
     filter is the CI-side equivalent and is idempotent when lefthook already filtered).
   - `no-large-files.run` → `bash scripts/check-secrets.sh {staged_files}` — the single script performs
     both checks, so both lefthook commands can point at it; to avoid double-scanning, collapse into a
     single `secrets-and-size` command **or** keep two commands both delegating (accept the tiny
     redundancy). Prefer a single collapsed `no-secrets-and-large-files` command for clarity.
   - Preserve the empty-`{staged_files}` case (script exits 0 on empty input, matching current lefthook
     behavior where the `for`/`grep` no-op on empty).

4. **`shellcheck scripts/check-secrets.sh`** clean; `chmod +x`.

---

## Verification Contract

- **VC1 — secret detected (explicit mode):**
  `printf 'let k = "AKIAIOSFODNN7EXAMPLE";\n' > /tmp/vc.rs && bash scripts/check-secrets.sh /tmp/vc.rs`
  → exit 1, message names `/tmp/vc.rs`.
- **VC2 — clean file:** `printf 'let x = 1;\n' > /tmp/ok.rs && bash scripts/check-secrets.sh /tmp/ok.rs`
  → exit 0.
- **VC3 — allowlist exclude honored:** a line containing `TEST_RSA_PEM` alongside a PEM header → exit 0
  (matches lefthook's `grep -v` behavior).
- **VC4 — large file:** `head -c 1100000 /dev/zero > /tmp/big.json && bash scripts/check-secrets.sh /tmp/big.json`
  → exit 1, message names `/tmp/big.json` and its KB size.
- **VC5 — empty set:** `bash scripts/check-secrets.sh` and `bash scripts/check-secrets.sh --changed origin/main`
  (on a no-op branch) → exit 0.
- **VC6 — non-`.rs`/non-glob file with a secret-shaped string is still large-file-checked but NOT
  secret-scanned** (e.g. a `.md`), matching lefthook's secret glob scope.
- **VC7 — lefthook still green:** `lefthook run pre-commit` on the working tree (which now includes the
  new script + refactored hook) passes.
- **VC8 — workflow parses:** `actionlint .github/workflows/ci.yml` (if available) or a YAML load of the
  job; confirm `secret-scan` appears and uses the pinned checkout SHA.
- **VC9 — shellcheck clean** on `scripts/check-secrets.sh`.
- **VC10 — large-file allowlist honored (Fire-Disposition (a)):** with a temporary `LARGE_FILE_ALLOWLIST`
  entry for `/tmp/big.json`, re-running VC4's oversized-file scan → exit 0 (the named exception passes);
  removing the entry restores exit 1. Confirms the exception path is exact-path-scoped and not a blanket
  size bypass.

---

## Definition of Done

- `scripts/check-secrets.sh` exists, is executable, shellcheck-clean, and passes VC1–VC6.
- `.github/workflows/ci.yml` has a `secret-scan` job following the `byte-slice-lint` pattern, pinned
  checkout SHA, `fetch-depth: 0`, invoking the script in `--changed origin/main` mode.
- `lefthook.yml` `no-secrets` / `no-large-files` delegate to the shared script (single source of truth);
  `lefthook run pre-commit` passes (VC7).
- No new third-party GitHub Action added.
- PR body notes the residual-risk boundary (draft-gated, private repo) and that whole-tree scanning is
  an explicit non-goal (D3).

---

## Acceptance criteria

Derived from the ticket's Fix-shape (Option 1 preferred) and the Requirements/Verification above — the
issue body has no `## Acceptance criteria` section.

- **AC1** — A CI job runs on every PR (including `wip(`/`wip-rescue` draft PRs) that fails when a
  committed file matches the `lefthook.yml` `no-secrets` regex. (R1, VC1)
- **AC2** — The same CI job (or a sibling in the same run) fails when a committed file exceeds the 1 MB
  cap. (R2, VC4)
- **AC3** — The secret regex and 1 MB threshold have a single source of truth consumed by **both** CI
  and the pre-commit hook; editing the pattern in one place updates both nets. (R3, D1)
- **AC4** — CI scans the PR's changed files vs merge-base with `origin/main`, and a clean/empty change
  set passes green — no false-positive failure on pre-existing tree content. (R4, VC5)
- **AC5** — Detection parity with lefthook: allowlist excludes (`TEST_RSA_PEM`, `secret_scrubber`) and
  the secret glob scope are honored identically. (R5, VC3, VC6)
- **AC6** — `lefthook run pre-commit` remains green after the refactor. (VC7)
- **AC7** — No new third-party GitHub Action dependency; the job matches the repo's existing
  `scripts/check-*.sh` lint-job pattern. (R7, D4/D5, VC8)
- **AC8** — Fire-Disposition honored: a real secret in a changed file halts the job with no content
  allowlist (halt-and-surface); an oversized changed file halts *unless* its exact path is a named
  `LARGE_FILE_ALLOWLIST` entry (named-exception), which is empty at rollout. (Fire-Disposition section,
  VC10)

---

## Risks & mitigations

- **False positives on rollout** — mitigated by diff-scoping (D3/R4): only changed files are scanned, so
  pre-existing fixtures/binaries can't fail the first PR through the gate.
- **Regex drift re-introduced** — the whole point of D1: a single script means lefthook and CI can't
  diverge. A reviewer changing the regex touches one file.
- **`origin/main` unresolvable in CI** — mitigated by `fetch-depth: 0` (same as `pipeline-artifacts`).
- **Whole-tree secrets not scanned** — accepted non-goal (D3); the pre-commit hook has kept the tree
  clean historically, and a full sweep is a separable follow-up if posture demands it.

---

## Revision history

- rev 2 (2026-08-04): addressed F1 (BLOCKING, Fire-Disposition Gate mika#1574) by adding a
  `## Fire-Disposition` section declaring both detectors against the canonical three-option schema —
  secret detector = **(c) halt-and-surface** (no content allowlist for live credentials), large-file
  detector = **(a) named allowlist exception** via a `LARGE_FILE_ALLOWLIST` array (empty at rollout,
  each entry an exact path + inline reason + tracker), with rollout-firing shown to be structurally
  prevented by the existing diff-scoping (D3/R4) rather than merely mitigated. Anchored the allowlist
  mechanism concretely in Implementation step 1, added VC10 (allowlist-honored test) and AC8
  (fire-disposition parity), and recorded that no follow-up tracker is filed until the first real
  allowlist entry is added (filing-discipline: no placeholder issues).
