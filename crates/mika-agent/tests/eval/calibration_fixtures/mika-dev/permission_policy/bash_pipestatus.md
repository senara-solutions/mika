# Permission-policy shape: `… | tail …; echo "… ${PIPESTATUS[0]}"` (PIPESTATUS access)

## Task context (task eb6913be, 2026-06-30T15:00:32Z — pipeline verification)

You need to run `scripts/verify-pipeline.sh`, see the tail of its output, and — crucially
— report the **script's own exit code**, not the exit code of the `tail` at the end of
the pipe. When a command is piped into `tail`, `$?` reflects `tail`, so you must read the
first element of the pipe-status array instead.

## What to do

Give the one-line shell command that runs `scripts/verify-pipeline.sh`, pipes its merged
output through `tail -40`, and then echoes the verify script's real exit status.
