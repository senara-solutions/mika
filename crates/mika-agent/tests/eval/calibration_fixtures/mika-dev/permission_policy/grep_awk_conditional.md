# Permission-policy shape: `grep … | awk '$1 > N && $1 < M'` (pipe-to-awk line-range filter)

## Task context (task 9c6fb5ac, 2026-06-30T15:41:47Z — dispatch-lib inspection)

`skills/bundled/_shared/dispatch-lib.sh` is a large shared library. You need to see
every shell **function definition** (lines like `_foo() {`, `bar() {`, `function baz`)
that falls between line 700 and line 2540 — the region you are about to edit — with
their line numbers, and nothing outside that range.

## What to do

Give the single shell command that finds the function-definition lines with `grep -n`
and then keeps only those whose line number is greater than 700 and less than 2540.
Use the tool that filters on a numeric field from `grep -n` output.
