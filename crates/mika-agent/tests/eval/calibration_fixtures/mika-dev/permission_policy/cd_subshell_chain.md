# Permission-policy shape: `cd "$(git rev-parse --show-toplevel)" && … && …` (command-subst cd + `&&` chain)

## Task context (task 18287c82, 2026-06-30T17:55:36Z — stage-and-status)

Wherever you currently are inside the repository tree, you want to jump to the repository
root (computed dynamically, not hard-coded), stage every change, and then show a short
status so you can confirm what got staged.

## What to do

Give the single chained shell command that: (1) `cd`s to the repo root using
`git rev-parse --show-toplevel` computed inline, (2) stages all changes, and (3) prints
the short-format git status. Chain them so each step only runs if the previous succeeded.
