# Applicable criteria for this change

These are distilled from the project's active conventions. They are additive
context, not the complete standards contract.

## Shell substrate (`skills/bundled/_shared/dispatch-lib.sh`)

- The file runs under `set -euo pipefail`. Any new predicate must not abort the
  script when it legitimately evaluates false.
- A signal that cannot be read is NEVER a satisfied term: unreadable metadata, a
  missing file, an unreadable mtime must take the subject OUT of the treated
  population, never into it. Fail-safe direction is stated per site.
- Structural guards in the sibling test harness count call sites by grep
  (`claude-pilot` launch sites, `--log-dir "$_PILOT_LOG_DIR"` occurrences,
  `rm -rf` on worktree paths, `worktree remove --force`). A new call site that
  does not update those counts breaks the suite; a count raised without cause
  silently widens the guard.
- Every read of `$_PILOT_LOG_DIR` must call `_pilot_log_dir` on the SAME line
  (co-location invariant, enforced by a test).
- Constants embedded in double-quoted strings must escape backticks: an
  unescaped backtick is command substitution.

## Observability discipline

- A log event consumed by an operator must have exactly one writer, so its
  absence is itself information.
- An event must be emitted at a level actually collected. DEBUG is not collected
  on this deployment.
- An event of pure observability must not be re-read by any branch of the same
  file; promoting one into a control signal is a contract change.

## Guard design

- A budget/counter must be armed before the action it bounds, so a downstream
  failure cannot reopen it.
- A predicate whose input source is not named leaves the implementer a choice the
  design had the means to settle. Reading a file the guard itself writes makes
  the predicate self-sustaining and true by construction.

## Test harness (`skills/bundled/_shared/test-dispatch-lib.sh`)

- Assertions are inline (`assert_eq` / `assert_contains` / `assert_not_contains`),
  not `test_*()` functions, in the two most recent sections.
- A behavioural probe must neutralise only the launch of the external process and
  run the real function, so that what is measured comes from the code under test.
- A negative control that could pass for the wrong reason (e.g. a fixture below a
  size filter, so the function returns "not found") is worse than no test.
