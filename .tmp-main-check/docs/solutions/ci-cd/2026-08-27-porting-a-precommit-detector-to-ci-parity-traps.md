---
title: "Porting a pre-commit detector into CI: parity lives in the command's output shape, not its regex"
date: 2026-08-27
category: ci-cd
module: ci
component: check-secrets
tags: [ci, lefthook, pre-commit, secret-scan, grep, git-diff, detection-parity, false-negative, no-verify]
problem_type: security_issue
issue: "mika#1689"
---

# Porting a pre-commit detector into CI: parity lives in the command's output shape, not its regex

## Why the port happened at all

mika#1685 added `git commit --no-verify` to the three post-flight rescue commit sites in
`skills/bundled/_shared/dispatch-lib.sh`, so a leftover clippy nit could no longer wedge the salvage
path. `--no-verify` is **all-or-nothing**: it skips the entire lefthook `pre-commit` block, and that
block mixed lint (`rust-fmt`, `rust-clippy`, typechecks) with two gates that are not lint —
`no-secrets` and `no-large-files`. The rescue path pushes its branch to origin, so from mika#1685
onward a secret in pilot-written code could reach the remote with no automated net.

**The general shape:** a bypass flag that is all-or-nothing turns any pre-commit block mixing lint
with security gates into a single unit. You cannot skip half of it. The security half has to live at
a layer the bypass cannot reach — for us, CI — and the detection rule then has to exist in exactly
one place, or the two nets drift.

The fix (mika#1689) extracts the regex and the 1 MB cap into `scripts/check-secrets.sh`, consumed
both by lefthook (`{staged_files}`) and by a new `secret-scan` CI job (`--changed origin/main`).

## The two parity traps — both found by reviewing the new gate against the old one, both reproduced before fixing

### 1. A per-file loop silently narrows a `grep -v` exclusion

The lefthook command was:

```yaml
run: >
  grep -rn -E '<regex>' {staged_files}
  | grep -v 'TEST_RSA_PEM'
  | grep -v 'secret_scrubber'
```

With **multiple file arguments**, `grep` prefixes every output line with `<path>:`. So those
`grep -v` terms filtered on the **path** as well as the content — and one of them was, in practice,
a whole-file exclusion: `crates/mika-agent/src/secret_scrubber.rs`, whose test fixtures are
deliberately secret-shaped, passed as a file because its *path* contained `secret_scrubber`.

The ported script scanned file-by-file (`grep -nE "$SECRET_REGEX" "$f"`), whose output is
`<line>:<content>` — no path. The exclusion silently narrowed to content-only, and the fixtures
started firing: **6 violations on a file the branch never touched**, meaning every PR touching the
scrubber's own tests would have failed both the CI job and the pre-commit hook.

Fix — re-prefix the path before the excludes, reproducing the multi-file `grep -rn` shape:

```bash
match="$(grep -nE "$SECRET_REGEX" "$f" 2>/dev/null \
    | sed "s|^|$f:|" \
    | grep -v 'TEST_RSA_PEM' \
    | grep -v 'secret_scrubber' || true)"
```

**Transferable rule:** when porting a detector, diff it against the *old command's output shape*,
not just its pattern. Pipeline filters downstream of `grep` see whatever `grep` chose to print, and
that depends on how many file arguments it got.

### 2. `git diff --name-only` C-quotes non-ASCII paths — and a "skip what I can't stat" guard turns that into a false negative

`git diff --name-only` renders a path containing non-ASCII bytes as a C-quoted string:

```
"s\303\251cr\303\250t-accentu\303\251.rs"
```

That string names no file on disk. The scanner's belt-and-suspenders guard (`[[ -f "$f" ]] || continue`,
there to tolerate deletions surviving `--diff-filter=ACM`) then skipped it — and reported the change
set **clean**. Verified end to end: a file named `sécrèt-accentué.rs` carrying an `AKIA…` key passed
`--changed` before the fix and fails it after. In a repo whose docs and logs carry French accents,
this is not a hypothetical path.

Fix — read the diff NUL-delimited, where git does no quoting at all:

```bash
while IFS= read -r -d '' f; do
    [[ -n "$f" ]] && FILES+=("$f")
done < <(git diff -z --name-only --diff-filter=ACM "$merge_base"..HEAD)
```

**Corollary, and the reason this one is worth the doc:** in a *detector*, a silent skip is
indistinguishable from a clean scan. Announce the skip, and count the files actually examined rather
than the size of the candidate set — otherwise `scanned N — clean` overstates coverage by exactly the
files the detector failed to open.

## What generalizes

- A `--no-verify`-style escape is all-or-nothing. Never colocate a security gate with lint behind one.
- Single-source the rule (one script, two callers) or the two nets drift on the next regex edit.
- Diff-scope a newly-introduced gate (`--diff-filter=ACM` vs the merge-base) so it cannot fire on
  pre-existing tree content. This is also what makes an *empty* named-exception allowlist honest at
  rollout: nothing already in the tree can force an entry.
- Verify parity by running the **old** and **new** commands against the same real files in the repo.
  Both defects here were invisible to the plan's Verification Contract, which tested the new scanner
  against synthetic fixtures in `/tmp` — the fixtures had ASCII names and no path-scoped exclusion.
