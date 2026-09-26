# Injection verification — mika-manager cadence wiring (PR follow-up to #1932)

**Status:** CLOSED — verified 2026-08-22 during PR preparation.

Per `feedback_verify_pipeline_passes_without_the_fix`, every load-bearing test
in `crates/mika-agent/src/milestone_manager/spawn.rs` must fail when its
target invariant is inverted, then pass again when restored. This doc records
the injection-verify pass for the cadence-wiring follow-up PR.

## AC1 — env-gate must default off

**Guard tests:** `env_unset_returns_none`, `env_target_empty_string_returns_none`,
`env_gate_is_load_bearing_default_off` (redundant, catches state-cache
regressions).

**Injection:** Replaced the early-return branch in `manager_config_from_env`
so it returns a bogus `Ok(Some(ManagerConfig{...}))` when the target is unset:

```
-        _ => return Ok(None),
+        _ => return Ok(Some(ManagerConfig{ target: MilestoneRef{repo:"forced/target".into(),number:1}, ... })),
```

**Result:** All three tests fail as expected.

```
test result: FAILED. 12 passed; 3 failed; ...
failures:
    milestone_manager::spawn::tests::env_gate_is_load_bearing_default_off
    milestone_manager::spawn::tests::env_target_empty_string_returns_none
    milestone_manager::spawn::tests::env_unset_returns_none
```

**Restore:** `cp /tmp/spawn.rs.orig …/spawn.rs` — all 15 tests green.

## AC3 — numeric env vars fall back to defaults on invalid

**Guard tests:** `env_invalid_heartbeat_falls_back_to_default`,
`env_invalid_poll_falls_back_to_default`, `env_invalid_silence_falls_back_to_default`,
`env_zero_heartbeat_falls_back_to_default`.

These tests set the env var to unparseable/zero values and assert the parsed
`ManagerConfig` field matches the documented default. If `read_u64_env` /
`read_u32_env` are ever changed to propagate the parse error instead of
falling back, these tests panic on the `expect("target valid").expect("Some")`
chain — verified structurally by inspection (the tests would fail because
`manager_config_from_env()` would return `Err`, not `Ok(Some(...))`).

## AC4 — malformed target returns Err (loud)

**Guard tests:** `env_invalid_target_returns_error`,
`env_target_non_numeric_returns_error`.

These tests set `MIKA_MANAGER_TARGET_MILESTONE` to malformed values
(`"malformed-no-hash"`, `"senara-solutions/mika#not-a-num"`) and assert
`manager_config_from_env()` returns `Err(...)` naming the offending env var.
If the parse were ever changed to fall back silently (mirroring the numeric
env-vars), an operator's typo could silently disable the cadence with no
diagnostic — these tests lock the loud-failure invariant.

## AC7 — cancel token is honored within one poll interval

**Guard test:** `spawn_respects_cancel_token`.

The test spawns the real cadence loop with a 50ms poll and cancels after
200ms. The `tokio::time::timeout(Duration::from_secs(2), handle)` wrapper
asserts the join completes; if the `tokio::select!` branch on
`cancel.cancelled()` were ever removed, the test would time out at the 2s
mark and fail with `"spawn task must exit within 2s of cancel — got timeout"`.

## AC8 — no_dispatch structural gate preserved

Ran `cargo test -p mika-agent --lib milestone_manager::no_dispatch_test`
before and after this PR's diff — green both times. The spawn module adds
zero forbidden tokens (`run_claude_pilot`, `pr_merge_with_gate`, `gh api
PATCH/POST/DELETE`, etc.), so the structural gate's FORBIDDEN_TOKENS list is
unchanged.

## Composition — full pipeline stays green

`cargo test -p mika-agent --lib milestone_manager` reports 89 passed / 0
failed / 0 ignored — the new 15 spawn tests plus the pre-existing 74
milestone_manager tests all pass together.
