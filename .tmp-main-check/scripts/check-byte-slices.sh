#!/usr/bin/env bash
# CI lint: byte offsets into text must land on UTF-8 character boundaries.
#
# THE PROPERTY, not a list of syntaxes:
#   Any operation that indexes a `String`/`&str` by a *computed byte offset*
#   panics when that offset falls inside a multi-byte character. Rust spells
#   this hazard many ways — `&s[..n]`, `s.truncate(n)`, `s.split_at(n)`,
#   `s.split_off(n)`, `s.insert(n, c)`, `s.remove(n)`, `s.drain(..n)` — and
#   every one of them asserts `is_char_boundary` and aborts the process.
#
# See mika#764 (the founding incident, slice syntax) and mika#2103 (the
# recurrence: `String::truncate` panicked 26 times in two days, taking the
# webhook drain down with it, because this lint only knew the slice spelling).
#
# WHEN YOU EXTEND THIS SCRIPT, EXTEND IT BY PROPERTY.
#   The mika#2103 lesson is that a guard which knows one *writing* of a defect
#   lets every other writing through. If you find a new way to compute a byte
#   offset into text, it belongs here — do not wait for it to panic in prod.
#
# THE FIX is `mika_common::text::safe_truncate(s, n)`, which floors to a
# boundary and never panics. Prefer it over re-deriving a boundary walk.
#
# ALLOWLIST: a line carrying `// safe-byte-slice: <reason>` is exempt. The
# reason is mandatory in spirit — it is what a future reader needs in order
# to re-audit the site. Sites with no character-boundary concept at all
# (`Vec::truncate`, `OpenOptions::truncate(bool)`) are exempted this way too:
# annotating them once is the price of a lint that reads the property rather
# than guessing string-ness from a variable name.
#
# Exit 0 if clean, exit 1 with actionable errors if violations found.

set -euo pipefail

VIOLATIONS=0
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Scan root. Defaults to the repo's crates/; an explicit argument lets the
# anti-vacuity harness point the guard at a fixture tree (mika#2103).
SCAN_ROOT="${1:-$REPO_ROOT/crates}"

# report <pattern-label> <grep-output-line>
report() {
    echo "ERROR: unsafe byte-offset pattern ($1) at $2"
    VIOLATIONS=$((VIOLATIONS + 1))
}

# Common exclusions applied to every pattern.
filter_common() {
    grep -v '// safe-byte-slice:' | grep -v '/target/' || true
}

# ── Pattern A: [..var.len().min(N)] — truncation via min() without a boundary
# check. Always unsafe on &str: `.len()` is bytes and `.min(N)` can land inside
# a multi-byte character.
while IFS= read -r line; do
    [[ -n "$line" ]] && report "Pattern A: slice via .len().min()" "$line"
done < <(grep -rn '\.len()\.min(' "$SCAN_ROOT" --include='*.rs' \
    | grep '\[\.\..*\.len()\.min(' | filter_common)

# ── Pattern B: &str_var[..LITERAL_INT] — direct byte offset with a literal.
# Name-scoped to known string-typed variables to avoid flagging &[u8] indexing.
while IFS= read -r line; do
    [[ -n "$line" ]] && report "Pattern B: slice at literal byte offset" "$line"
done < <(grep -rn -E '&(content|body|bad_output|cleaned|output|msg\.content|second_output|chunk_context)\[\.\.([0-9]+)\]' \
    "$SCAN_ROOT" --include='*.rs' | filter_common)

# ── Pattern C: .truncate(<non-boolean>) — the mika#2103 class.
#
# Deliberately NOT scoped by variable name. A name allowlist (as in Pattern B)
# would reproduce the very failure being fixed: it cannot see a `String` that
# happens to be called something the list never anticipated. So every
# `.truncate(N)` in crates/ must either be char-boundary-safe by construction
# or carry a `// safe-byte-slice:` reason — `Vec::truncate` included.
#
# `truncate(true)` / `truncate(false)` (std::fs::OpenOptions) are excluded by
# the argument shape: a bool literal is never a byte offset.
while IFS= read -r line; do
    [[ -n "$line" ]] && report "Pattern C: .truncate() at a byte offset" "$line"
done < <(grep -rnE '\.truncate\(' "$SCAN_ROOT" --include='*.rs' \
    | grep -vE '\.truncate\((true|false)\)' | filter_common)

# ── Pattern D: byte-offset cutting and mutation with a computed index.
#
# `split_at` / `split_off` / `drain(..n)` where the index is derived from
# `.len()` or arithmetic — the mika#2103 `logs.rs` shape,
# `input.split_at(input.len() - 1)`, which panics on a multi-byte final char.
while IFS= read -r line; do
    [[ -n "$line" ]] && report "Pattern D: computed byte offset in split/drain" "$line"
done < <(grep -rnE '\.(split_at|split_at_mut|split_off|drain)\(' "$SCAN_ROOT" --include='*.rs' \
    | grep -E '\.(split_at|split_at_mut|split_off|drain)\([^)]*(\.len\(\)|[0-9] *[-+]|[-+] *[0-9])' \
    | filter_common)

# ── Pattern E: String::insert(idx, 'c') — recognised by its char-literal
# second argument, which isolates it from the many map/set `insert` calls.
while IFS= read -r line; do
    [[ -n "$line" ]] && report "Pattern E: String::insert at a byte offset" "$line"
done < <(grep -rnE "\.insert\([^,)]+, *'" "$SCAN_ROOT" --include='*.rs' | filter_common)

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "Found $VIOLATIONS unsafe byte-offset pattern(s)."
    echo "These panic on multi-byte UTF-8 (accents, em-dashes, emoji) — see #764 and #2103."
    echo "Use mika_common::text::safe_truncate(), or annotate the line with"
    echo "  // safe-byte-slice: <why this offset is always a char boundary>"
    exit 1
fi

echo "No unsafe byte-offset patterns found."
exit 0
