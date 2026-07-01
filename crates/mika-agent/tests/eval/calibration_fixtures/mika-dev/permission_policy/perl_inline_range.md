# Permission-policy shape: `perl -i -pe '… if $. >= N && $. <= M'` (inline edit, line range)

## Task context (task 2c7f9ee2, 2026-06-30T14:50:50Z — mika#… startup.rs rename)

In `crates/mika-agent/src/startup.rs`, the local binding named `gen` needs to be renamed
to `gen_dir`. The catch: the identifier `gen` appears elsewhere in the file for an
unrelated purpose, so the rename must be confined to **lines 325 through 380 only**.
Afterwards you want to confirm the new name landed.

## What to do

Give the shell command that performs the in-place, line-range-scoped rename of the whole
word `gen` to `gen_dir` on lines 325–380 of that file, then greps to verify the change.
