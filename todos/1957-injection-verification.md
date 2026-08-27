# Injection verification — mika#1957 (shell-exec L3 hardening)

Per `feedback_verify_pipeline_passes_without_the_fix`: a gate that passes with the fix
removed is not a gate. Two inversions, run on branch
`fix/1957/shell-exec-cli-bypass-hardening-l3-gate`, both restored afterwards.

Plan inversion 1 ("Tier 1 fires") was dropped with Tier 1 itself — see the plan's
§ Corrections post-grooming C1.

## Inversion 1 — remove the L3 hardening block from `run.sh`

Deleted the whole `# --- shell-exec L3 hardening ... # --- end` block, kept the
pre-existing `FIRST_WORD` case.

```
test result: FAILED. 8 passed; 12 failed
```

The 12 failures are exactly the bypass-shape cases plus the documented deliberate
false-positive. The 8 survivors are the two first-word regressions (the old gate still
fires on them) and the six happy-path guards (unaffected by the scan's absence) — which
is the correct partition: the new tests fail *only* because the new gate is gone.

## Inversion 2 — revert to the regex the groomed plan proposed

Swapped the implemented boundary class back to the plan's original
`'(^|[[:space:]|;&`$(])(gws|gh)([[:space:]]|$)|/gws[[:space:]]|/gh[[:space:]]'`.

```
test result: FAILED. 15 passed; 5 failed
```

Failures: `shell_exec_rejects_sh_c_gws`, `shell_exec_rejects_sh_c_gh`,
`shell_exec_rejects_bash_c_gws`, `shell_exec_rejects_eval_gws`,
`shell_exec_rejects_piped_echo_sh`.

Every one of them is a quoted-subshell shape. The plan's leading boundary class omits
`'` and `"`, and in `sh -c 'gws ...'` the character immediately before `gws` is a quote,
so the alternation never fires. That is bypass shape 1 — the shape this ticket exists to
close — which is why the regex was corrected (plan § Corrections C2) rather than shipped
as written. This inversion is retained as a standing guard against re-introducing the
narrower class.

## Control — fix restored

```
test result: ok. 20 passed; 0 failed
```
