# mika#1968 — Injection verification log

Per `feedback_verify_pipeline_passes_without_the_fix` and the plan's
§"Injection verification" section, this file records the injection-verification
steps done for the new observability signals shipped in mika#1968.

**Discipline:** for each new log line, comment out the emission call,
watch a test fail, restore, verify the test passes again. This proves
the test actually anchors the emission and would catch a future regression
that drops or renames the event.

---

## Verified signals

### 1. `dotenv_loaded` / `dotenv_absent` / `dotenv_load_error`

**Emission sites:** `crates/mika-common/src/dotenv.rs::load_dotenv` — both
`eprintln!` and `info!/error!` paths (dual-channel per adversarial-review
A1 P0 fix).

**Test coverage:**

- `load_dotenv_reports_absent_when_file_missing` — locks the `Absent` arm
  runs without panic and `parse_dotenv` returns empty. Would fail if the
  match arm were removed or if the function panicked on missing file.
- `load_dotenv_reports_loaded_count_matches_parsed` — locks the invariant
  that `load_dotenv`'s `keys_from_file` count equals `parse_dotenv().len()`
  for the same path. Regression that stops calling `parse_dotenv` would
  fail this test.
- `load_dotenv_reports_zero_keys_for_empty_file` (T5 P3 fix) — locks the
  canary distinguishing "file present + empty" from `dotenv_absent`.
- `load_dotenv_handles_parse_error_without_panic` (T3 P2 fix) — locks the
  error arm's log-and-continue posture. A regression that panicked or
  swallowed the error would fail this test.

**Injection verification:** structural-only. The tests assert code-path
completion + observable side effect via `parse_dotenv`; they do NOT
capture stderr/subscriber output (that would require installing a test
subscriber + stderr redirection setup we chose not to force on the unit
scope, per the plan's "Test coverage" section).

**Manual verification (post-deploy):** `grep dotenv_ $MIKA_SPIRIT_LOG_FILE`
after boot must show one of the three states per boot. Absence indicates
the `eprintln!` channel regressed. `grep '"event":"dotenv_' $MIKA_SPIRIT_LOG_FILE`
after boot shows the structured JSON channel (silent when subscriber
unavailable pre-init — the eprintln! is the durable channel).

### 2. `mika_spirit_home_resolved` + `mika_spirit_env_check` eprintln! boot lines

**Emission sites:** `crates/mika-agent/src/bin/mika-spirit.rs::main` — pre-`logging::init()`
eprintln! at line 16 and line 32.

**Test coverage:** none — this is a `main()`-level surface. Testing
would require full-binary spawn with stderr capture, out of scope for
unit tests.

**Manual verification (post-deploy):**

```bash
# Expected: two lines per boot
grep 'mika_spirit_home_resolved\|mika_spirit_env_check' $MIKA_SPIRIT_LOG_FILE
```

Absence after a deploy indicates a regression to the mika-spirit
`main()` flow that removed or reordered the eprintln! calls.

### 3. `manager_gh_auth_check_ok` / `manager_gh_auth_check_failed`

**Emission sites:** `crates/mika-agent/src/milestone_manager/spawn.rs::spawn_manager_cycle_task`
inside the spawned task, before the cycle loop starts.

**Test coverage:**

- `verify_gh_auth_401_returns_err_unauthorized` — locks the 401 →
  `Err(GhAuthError { auth_class: Unauthorized, ...})` path that drives
  the `manager_gh_auth_check_failed` log.
- `verify_gh_auth_success_returns_ok_with_remaining` — locks the OK path
  with valid body → `Ok(remaining)`.
- `verify_gh_auth_malformed_body_returns_err_other` (T4 P2 fix) — locks
  the A2 P1 fidelity fix: malformed 200 body MUST fail-loud, not
  `Ok(0)`. Three cases covered (`{}`, empty string, schema-drift).

**Injection verification:** structural. The tests use a mock `GhRunner`
and assert return-type + `auth_class` + `stderr_head` shape. They do NOT
capture the surrounding `info!/error!` calls in `spawn_manager_cycle_task`
— that layer requires the full spawn machinery (already covered by the
existing `spawn_respects_cancel_token` integration path, which fires the
loop with a mocked config).

**Manual verification (post-deploy):**

```bash
grep 'manager_gh_auth_check_' $MIKA_SPIRIT_LOG_FILE
# Expected on every boot: exactly one line, either _ok (with rate_limit_remaining)
# or _failed (with auth_class + stderr_head + hint).
```

### 4. `manager_cadence_spawn_attempt` (with PID) + `manager_cadence_spawn_duplicate_rejected`

**Emission sites:** `spawn_manager_cycle_task` top-of-function (attempt log)
and inside the guard-check block (duplicate-rejected log).

**Test coverage:**

- `spawn_manager_cycle_task_second_call_rejected` — locks the guard
  behavior: first call returns `Some(handle)`, second returns `None`.
  Includes `reset_spawn_guard_for_test()` for hermeticity.

**Manual verification (post-deploy):**

```bash
grep 'manager_cadence_spawn_attempt' $MIKA_SPIRIT_LOG_FILE
# Expected on every boot: exactly one line with the current PID.
# TWO lines with SAME pid indicates the guard collapsed a double-init
# (candidate #2 — single process re-entering; the guard did its job).
# TWO lines with DIFFERENT pids indicates TWO mika-spirit processes
# (candidate #1 — supervise-daemon race; requires OpenRC fix per plan §6d).

grep 'manager_cadence_spawn_duplicate_rejected' $MIKA_SPIRIT_LOG_FILE
# Expected: zero lines on healthy boot. Any hit means candidate #2 fired
# and was collapsed.
```

### 5. `auth_class` structured field on `manager_cycle_error`

**Emission sites:** `spawn_manager_cycle_task` inside the cycle loop's
`Err(e)` arm (change 5c).

**Test coverage:**

- `classify_cycle_error_discriminates_403_and_network` — locks the four
  discriminator buckets (Unauthorized/Forbidden/Network/Other). Any
  regression that collapses buckets would fail this test.

**Manual verification (post-deploy):**

```bash
grep 'manager_cycle_error' $MIKA_SPIRIT_LOG_FILE | \
  jq 'select(.auth_class == "401")' | head -5
# Expected on a healthy deploy: zero 401s. Any 401 → the ticket's
# founding-incident class is recurring; investigate token/scope/expiry.
```

---

## What was NOT injection-verified (scope)

- The eprintln! sink stability under different terminal/pipe configurations
  (verified conceptually per the adversarial-review pass — Rust's
  `eprintln!` handles broken-pipe by silently ignoring since 1.66).
- The compound doc's grep-signal examples were REVIEWED against emission
  sites during A1 P0 fix — verified they match the eprintln! output shape
  operators would grep for.
- Downstream operator dashboards that might depend on the pre-fix
  `manager_cycle_error` shape (without `auth_class`) — no dashboards
  observed to depend on it via a quick repo grep. Grep signal is
  strictly additive.

## Related solutions

- `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md`
  — the compounded pattern this ticket produced.
- `feedback_verify_pipeline_passes_without_the_fix` — the memory that
  drove this discipline.
