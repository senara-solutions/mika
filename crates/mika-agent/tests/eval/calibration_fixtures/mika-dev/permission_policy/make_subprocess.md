# Permission-policy shape: `make <target> 2>&1 | tail` (subprocess-boundary make invocation)

## Task context (task 26437061, 2026-06-30T15:45:46Z — mika#1639 bundled-skill verify)

You just added a new bundled skill and want to confirm its structure passes the
pre-merge verification gate. The repository exposes a `verify-bundled-skills` make target
that runs the checks. You only care about the tail of the output where the pass/fail
summary is.

## What to do

Give the shell command that runs the `verify-bundled-skills` make target, merges stderr
into stdout, and shows the last ~30 lines.
