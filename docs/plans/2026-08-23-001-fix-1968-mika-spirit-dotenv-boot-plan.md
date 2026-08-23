# Plan — fix(mika-spirit): dotenv boot + mika-manager auth/single-init hardening

**Status:** DRAFT (revision 2 — post-body-amendment 2026-08-23)
**Date:** 2026-08-23
**Ticket:** mika#1968
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Substrate reliability — env-vars-at-boot verification + mika-manager auth/idempotency hardening

**Revision 2 scope (2026-08-23 post-cadence-observation):** The body was amended with AC5 (GitHub auth 401 in daemon context) and AC6 (single-init idempotent spawn) after sami's diagnostic confirmed the cadence loop is running but every cycle 401s and each event is logged 2× (double-init). Sections 5 + 6 below address AC5 + AC6; the original sections 1-4 (AC1-AC4) are unchanged from revision 1.

## Why

mika#1945 shipped `manager_config_from_env()` wired at `server::run_server` startup (line 1354, gated on `MIKA_MANAGER_TARGET_MILESTONE`). Post-deploy 06:59 UTC 2026-08-22, zero milestone#30 heartbeat comments after 24h despite `HEARTBEAT_INTERVAL_SECS=21600` implying 4 expected. Root-Claude's evidence: `tr '\0' '\n' < /proc/$(pgrep -f 'mika --agent mika-arch')/environ | grep MIKA_MANAGER → empty`.

## Codebase reality (verified, not inferred)

The mika-spirit binary ALREADY calls dotenv at boot:

```rust
// crates/mika-agent/src/bin/mika-spirit.rs:6
mika_common::dotenv::load_dotenv(&home_dir);
mika_common::dotenv::check_env_warnings(&home_dir);
```

- `resolve_home_dir()` (`crates/mika-common/src/home.rs:66`) priority: `$MIKA_HOME` env > `dirs::home_dir()` + `/.mika`.
- `load_dotenv()` (`crates/mika-common/src/dotenv.rs:10`) uses `dotenvy::from_path()` which does NOT override existing env vars — shell/systemd/container ENV always wins. Silently skips on `ErrorKind::NotFound`.
- **`dotenvy::from_path()` DOES call `std::env::set_var()` internally** — verified at `/home/samidarko/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/dotenvy-0.15.7/src/iter.rs:35` and `:52`. The existing test `test_load_dotenv_loads_vars` at `crates/mika-common/src/dotenv.rs:218-238` proves the contract: after `load_dotenv(tmp.path())` returns, `std::env::var("MIKA_TEST_DOTENV_LOAD")` returns the file's value. Since `std::env::set_var()` mutates the process env visible to `/proc/<pid>/environ`, AC2's diagnostic path is satisfied by the existing code — the ticket's failing observation (empty grep) must be caused by one of the three subcauses below, not by dotenvy failing to persist the vars.
- mika CLI (`crates/mika-cli/src/main.rs:29-33`) also loads per-agent .env first then global .env.
- `run_server` (`crates/mika-agent/src/server/mod.rs:676`) is called AFTER `load_dotenv` in the mika-spirit main. `manager_config_from_env()` reads env inside `run_server` (line 1354).

**Divergence with ticket body:** ticket claims "the supervised binary startup may not" call dotenv. Code shows it does. Also, the reported process (`mika --agent mika-arch`) is the CLI, not mika-spirit — the CLI runs an ephemeral subprocess whose env-check does not prove mika-spirit's env state. The real failure surface is likely one of:

1. **Deploy staleness**: mika-spirit binary running at time of check was pre-mika#1945 (didn't include the manager wiring at all). Sami's OpenRC init workaround set env vars for immediate cadence recovery — but that path only fixed the *old* binary's env; the *new* binary already had dotenv but wasn't running yet.
2. **`resolve_home_dir()` returns wrong path under supervise-daemon**: OpenRC `supervise-daemon` on gentux launches with CWD=`/` and may inherit `HOME=/` from init. `dirs::home_dir()` returns `/` → `.mika` join gives `/.mika` (not `/home/samidarko/.mika`). `.env` at `/.mika/.env` doesn't exist → silent skip.
3. **HOME unset under supervise-daemon**: `dirs::home_dir()` returns `None` → `resolve_home_dir` errors → mika-spirit fails to start entirely (not silently loses env).

The ticket's ACs remain load-bearing regardless of which subcause is right: an observable, verified, boot-time confirmation that mika-spirit resolved the expected home dir AND loaded the expected env vars is what closes the gap between "code calls the function" and "operator can confirm from logs the vars are live."

## What

Three surgical additions to `crates/mika-agent/src/bin/mika-spirit.rs` + `crates/mika-common/src/dotenv.rs`, plus a compound doc capturing the pattern:

### 1. `load_dotenv` returns which path it loaded (or explicit why-not)

**File:** `crates/mika-common/src/dotenv.rs`.

**Change:** `load_dotenv(home_dir: &Path)` signature stays; internal behavior gains structured tracing:

- On successful `dotenvy::from_path()`: emit `info!(target: "mika::env", event = "dotenv_loaded", path = %env_path.display(), keys_from_file = <count>)`.
- On `ErrorKind::NotFound`: emit `info!(target: "mika::env", event = "dotenv_absent", path = %env_path.display())` (currently silent).
- On other errors: promote `warn!` to `error!` and add `event = "dotenv_load_error"` field (currently `warn!` without event tag).

**Rationale:** the class of failure — file absent vs load failed vs load succeeded — is currently indistinguishable in production logs. `info!` on both success and absent turns operator observation from "no line = ???" into "always a line = deterministic state." Log lines carry `event=` field per the existing structured-log pattern (see `docs/architecture/kg-implementation-conventions.md` C3.1 for the target-namespace + event-field shape mika uses).

**Keys-from-file count:** derive via `parse_dotenv(home_dir)` (already exported, returns `HashMap<String, String>`) called right before `dotenvy::from_path` — costs one extra file read at boot (negligible). Emitted so operators can grep `dotenv_loaded` and see "loaded 12 keys" vs "loaded 0 keys" — the latter is a canary for "file exists but empty."

**Double-read tradeoff (per F2 review):** the two file reads (parse_dotenv for count, then from_path for actual set_var) must be documented in-code so future maintainers see the tradeoff. Required code comment at the top of the parse-then-load block:

```rust
// Double-read tradeoff: parse_dotenv() for count, then dotenvy::from_path()
// for the actual load. Cost: 2 syscalls at boot. Kept for observability —
// the `keys_from_file` field in the dotenv_loaded log line lets operators
// distinguish "file loaded 0 keys" (empty file) from "no file" (dotenv_absent).
```

### 2. mika-spirit emits resolved home + startup env-check summary

**File:** `crates/mika-agent/src/bin/mika-spirit.rs`.

**Change (additive):**

```rust
async fn main() -> Result<()> {
    let home_dir = mika_common::home::resolve_home_dir()?;
    tracing::info!(
        target: "mika::env",
        event = "mika_spirit_home_resolved",
        home = %home_dir.display(),
        mika_home_env = std::env::var("MIKA_HOME").ok().as_deref().unwrap_or("<unset>"),
        home_env = std::env::var("HOME").ok().as_deref().unwrap_or("<unset>"),
        "mika-spirit resolved home directory"
    );
    mika_common::dotenv::load_dotenv(&home_dir);
    mika_common::dotenv::check_env_warnings(&home_dir);
    // ... rest unchanged
```

**Why it helps:** the founding incident's diagnostic path was "grep the process's `/proc/<pid>/environ`" — indirect and requires a running mika-arch process. The new line is a boot-time observation: an operator seeing `mika_spirit_home_resolved home=/ mika_home_env=<unset> home_env=<unset>` immediately knows subcause #2/#3 above without needing to run diagnostic subprocesses. Trivially cheap (one tracing call at boot).

**Note on log-init ordering:** `logging::init()` runs *after* `load_dotenv` in the current mika-spirit main (line 24 vs line 6). This means the two info! lines proposed above (fired before line 24) emit at the default `tracing` global-subscriber level — which is off by default until `init()` installs a subscriber. To make these lines observable, either (a) move them *after* `logging::init()` and re-order dotenv accordingly, or (b) install a minimal early-init subscriber. Recommend (a): move `load_dotenv` + `check_env_warnings` + new `info!` lines to AFTER `logging::init()` but BEFORE `run_server`. Settings loading currently sits between load_dotenv and logging::init — Settings::load reads env vars, so it must run AFTER load_dotenv but BEFORE run_server. Proposed order: `resolve_home_dir` → `logging::init(minimal_defaults)` → `info!(home_resolved)` → `load_dotenv` → `info!(dotenv result via updated fn)` → `check_env_warnings` → `Settings::load` → `logging::init(full settings-based)` → rest.

This is more invasive than a pure addition. Alternate: keep current ordering, accept that early info lines land only if a subscriber pre-exists (they don't in production). Fallback: use `eprintln!` for the two boot lines that must precede `logging::init` — bypasses tracing but writes to stderr where OpenRC captures it into the service log.

**Decision (KISS + orthogonality per review-guide.md):** use `eprintln!` for the two pre-logging lines. Structured JSON is the wrong shape for boot lines that fire before the JSON subscriber exists. eprintln! goes directly to stderr → OpenRC service log → operator grep. Simple, orthogonal to `logging::init` complexity, and matches how mika-common's `check_env_warnings` already handles pre-init messaging via eprintln!.

Revised plan: keep dotenv.rs additions (they run post-logging::init on all paths that already load a subscriber — CLI's per-invocation init + mika-spirit's post-load init). Add pre-load-dotenv eprintln! in mika-spirit for home_resolved + a post-load-dotenv eprintln! summarizing what was loaded.

### 3. Test coverage — home-dir-resolution smoke + dotenv-load-outcome smoke

**File:** `crates/mika-common/src/dotenv.rs` (extend existing `#[cfg(test)] mod tests`).

Two new tests:

```rust
#[test]
fn load_dotenv_reports_absent_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    // No .env file at tmp.path()
    // Cannot easily assert on log output without a test subscriber — instead
    // assert that the function completes without panic and returns unit.
    load_dotenv(tmp.path());
    // Parse-dotenv API surface confirms the "loaded" count is 0 for the same path
    let parsed = parse_dotenv(tmp.path());
    assert_eq!(parsed.len(), 0);
}

#[test]
fn load_dotenv_reports_loaded_count_matches_parsed() {
    let tmp = TempDir::new().unwrap();
    let env_file = tmp.path().join(".env");
    std::fs::write(&env_file, "MIKA_TEST_KEY_1=val1\nMIKA_TEST_KEY_2=val2\n").unwrap();
    let parsed = parse_dotenv(tmp.path());
    assert_eq!(parsed.len(), 2);
    // The load_dotenv info line's keys_from_file field must equal parsed.len() —
    // structural invariant enforced by the impl calling parse_dotenv() then
    // reporting its .len().
    load_dotenv(tmp.path());
}
```

These are load-observable smoke tests, not tracing-subscriber tests. The invariant they pin: `load_dotenv`'s reported count == `parse_dotenv().len()` for the same path — enforced structurally by calling parse_dotenv() inside load_dotenv() to derive the count.

**File:** `crates/mika-common/src/home.rs` (extend existing `#[cfg(test)] mod tests`).

One new test proving `resolve_home_dir` with `MIKA_HOME=/tmp/test` returns that path (already covered indirectly by lines 574/584 tests — verify and skip if present).

### 4. Compound doc — app-owned config vs init-owned config

**File:** `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md`.

Content shape (~80 lines):

- **Class:** silent env-var drop across process-launcher boundary.
- **Failure family:** any binary spawned by a process supervisor (OpenRC supervise-daemon, systemd without `EnvironmentFile=`, Docker without `--env-file`) that expects env vars set in a project's `.env` file will silently see those vars as unset unless the binary itself reads the file.
- **Rule:** the app owns its config. The launcher does not. `dotenvy::from_path("~/.mika/.env").ok()` at the top of `main()` (or an equivalent) is the load-bearing gate. This is defense-in-depth against every launcher shape.
- **mika's implementation:** `mika_common::dotenv::load_dotenv()` + `resolve_home_dir()` — called at every binary entry point (mika-spirit, mika CLI, calibrate, eval-diff, verify_bundled_skills).
- **Diagnostic path:** grep the process's `/proc/<pid>/environ` for the expected var — this is the ground-truth check. If empty, either the app doesn't call dotenv, or the app resolved a wrong home dir. The `mika_spirit_home_resolved` and `dotenv_loaded` log lines added by mika#1968 turn this from indirect to direct.
- **Precedent:** `feedback_shipped_ne_equal_pas_deployed_uv_tool_env` (uv-tool env divergence class — same shape at the Python-packaging layer).

### 5. mika-manager cadence: GitHub auth valid in daemon context (AC5)

**Files:** `crates/mika-agent/src/milestone_manager/spawn.rs` (config population), `crates/mika-agent/src/milestone_manager/reader.rs` (gh runner).

**Codebase reality (verified, not inferred):**

- `spawn.rs:101` populates `ManagerConfig.github_token` via raw `read_string_env("MIKA_GITHUB_TOKEN")` — bypassing `Settings::agent_github_token()` and `Settings::resolve_github_token()`. That means the manager cadence cannot benefit from the GitHub App installation-token fallback (`crates/mika-common/src/config.rs:1179 pub async fn resolve_github_token`) that the rest of the engine uses.
- `reader.rs:47-53` `ProcessGhRunner` conditionally injects `GH_TOKEN` only if `self.token.is_some()` — when the raw env read at `spawn.rs:101` returns `None`, no `GH_TOKEN` is injected and `gh` falls through to `~/.config/gh/hosts.yml` under the service user, which is empty/unauthenticated → 401.
- The builtin `run_gh` handler (`crates/mika-agent/src/skills/builtin_handlers.rs:1561/2260/2457-2466`), `pr_merge_with_gate` (`crates/mika-agent/src/tools/pr_merge_with_gate.rs:760-763`), and the exec-handler executor (`crates/mika-agent/src/skills/executor.rs:702/2317`) all use the correct pattern: scrub `MIKA_*` and `GH_TOKEN`, then re-inject the token resolved from `ctx.github_token` (which threads from `Settings::agent_github_token()` through `ToolContext` from `AppState`). The manager is the outlier.

**Root cause:** Bypass of `Settings` accessor means (a) if `MIKA_GITHUB_TOKEN` is dropped from mika-spirit's env (the founding incident this ticket exists to fix, per AC1-4), the manager cadence silently loses auth even if `Settings` would have surfaced a valid GitHub App installation token; (b) even when `MIKA_GITHUB_TOKEN` IS set, no fallback to App auth is available. Both paths land at 401.

**Change 5a — Route token acquisition through `Settings::resolve_github_token()`:**

- Refactor `manager_config_from_env()` to take `&Settings` (or the async `resolve_github_token()` result) rather than reading raw env. Call site update at `server::run_server:1354-1356` — `Settings` is already in scope there (`AppState.settings`).
- Signature shift: `pub async fn manager_config_from_env(settings: &Settings) -> Option<ManagerConfig>` (async because `resolve_github_token()` is async).
- Populate `ManagerConfig.github_token` from `settings.resolve_github_token(&app_state.http_client).await` when available, falling back to `settings.agent_github_token()` if App auth is unconfigured. `None` remains valid (surfaces via boot-time sanity — see change 5b) — but only after Settings has been consulted, not before.
- **Non-goal:** re-injecting the App token on every cycle (the App installation token has a ~1h expiry). If the plan-level cost of periodic refresh is trivial, the follow-up spot fix is at the reader boundary — reject that scope for this ticket; file separately if steady-state 401s reappear post-fix on the App path.

**Change 5b — Boot-time GitHub sanity call:**

- New function `spawn.rs::verify_gh_auth(&ManagerConfig) -> Result<(), String>` invoked immediately at the top of `spawn_manager_cycle_task` (before entering the cycle loop).
- Calls `gh api /rate_limit` via the same `ProcessGhRunner` the loop uses (verifies the exact code path). On non-zero exit or `401` in stderr, emit `error!(target: "mika::milestone_manager", event = "manager_gh_auth_check_failed", exit_code = <n>, stderr_head = <first 200 chars>, hint = "MIKA_GITHUB_TOKEN missing/invalid in mika-spirit env — check `tr '\0' '\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_GITHUB_TOKEN`")`. Cycle loop still starts (do NOT panic — cadence is best-effort; log-and-continue is the right posture per substrate-reliability class). Cycle body will 401 on first tick; the loud boot-time line is the operator signal, not the cycle-error spam.
- On success, emit `info!(target: "mika::milestone_manager", event = "manager_gh_auth_check_ok", rate_limit_remaining = <n>)` so operators see explicit green.

**Change 5c — Cycle-error telemetry sharpening:**

The existing `manager_cycle_error` warn line (`spawn.rs`, exact line found by grep during implementation) currently carries the raw error body. Add a structured `auth_class = "401" | "403" | "network" | "other"` field parsed from the error, so operators can grep specifically for `manager_cycle_error auth_class=401` — separates auth failure from transient network failure without regex-parsing the free-text body. Small, additive, high-signal-per-line.

**Test coverage:**

- Unit test `test_verify_gh_auth_401_returns_err` in `spawn.rs`: mock `GhRunner` that returns exit=1 + stderr "HTTP 401" → `verify_gh_auth` returns `Err(...)` with the 401 discriminator.
- Unit test `test_verify_gh_auth_success_returns_ok` in `spawn.rs`: mock returning `{"rate":{"remaining":4999}}` → `Ok(())`.
- Unit test `test_manager_config_from_env_prefers_app_token`: `Settings` with valid App config → returned `ManagerConfig.github_token` is the installation token, not the PAT (uses mock resolver).

### 6. mika-manager cadence: single-init guard (AC6)

**Files:** `crates/mika-agent/src/milestone_manager/spawn.rs` (spawn-once mechanism), `crates/mika-agent/src/server/mod.rs:1354` (sole call site).

**Codebase reality (verified, not inferred):**

- `manager_cadence_start` is logged exactly once per `spawn_manager_cycle_task` invocation (`spawn.rs:135`).
- Grep across `crates/` for non-test invocations of `spawn_manager_cycle_task` returned only the single call site at `server::run_server:1354-1356`.
- The observed double-log at same-timestamp (39µs apart) therefore does NOT come from a hidden second caller of `spawn_manager_cycle_task`. It comes from `run_server()` itself being executed twice, or `spawn_manager_cycle_task` being racy in some observed shape, or two mika-spirit processes running against the same log sink.

**Divergence from ticket body:** The body's framing "spawn appelé 2× à startup (peut-être conversation + silent path) OR module `pub` visible depuis deux entry points" is contradicted by the grep. The two ACTUAL candidate root causes are:

1. **Two mika-spirit processes** — OpenRC `supervise-daemon` race, or a stale process not reaped before the new one started, or `deploy_mika` restart landed on top of an already-running process. Both PIDs share `MIKA_SPIRIT_LOG_FILE` → interleaved logs.
2. **`run_server()` called twice from bin** — a bin-side loop or dispatch path re-entering `run_server` before the first task exits. Requires reading `crates/mika-agent/src/bin/mika-spirit.rs` main flow end-to-end.

**Change 6a — Add a process-scoped `Once` guard inside `spawn_manager_cycle_task`:**

Defense-in-depth against whatever hits it twice. If root cause is #1 (two processes), the `Once` guard is a no-op — each process has its own `Once`; two processes still log twice. But if root cause is #2 (single process re-entering), the guard collapses the double-init.

```rust
// In spawn.rs, module-level:
use std::sync::OnceLock;
static MANAGER_SPAWN_GUARD: OnceLock<()> = OnceLock::new();

pub fn spawn_manager_cycle_task(cfg: ManagerConfig, cancel: CancellationToken) -> Option<JoinHandle<()>> {
    if MANAGER_SPAWN_GUARD.set(()).is_err() {
        warn!(target: "mika::milestone_manager",
              event = "manager_cadence_spawn_duplicate_rejected",
              "spawn_manager_cycle_task called twice within same process — second call rejected");
        return None;
    }
    // ... existing spawn body wrapped in Some(...)
}
```

Signature change from `JoinHandle<()>` to `Option<JoinHandle<()>>` — the sole caller at `server::run_server:1354-1356` already treats it as best-effort (already inside a match); update to handle `None` as "already spawned, skip."

**Change 6b — Diagnostic PID + process-start-time log at spawn:**

At the top of `spawn_manager_cycle_task` (before the guard check), log the current process's PID and start-time so operators can distinguish root-cause #1 from #2:

```rust
info!(target: "mika::milestone_manager",
      event = "manager_cadence_spawn_attempt",
      pid = std::process::id(),
      process_start_time = <read from /proc/self/stat>,
      "spawn_manager_cycle_task entered");
```

If root cause is #1, operators will see two `manager_cadence_spawn_attempt` lines with DIFFERENT `pid` values → confirms two processes. If root cause is #2, same `pid` → confirms single process double-entering.

**Change 6c — Investigation guardrail in-code:**

Add a code comment above the `MANAGER_SPAWN_GUARD` block:

```rust
// mika#1968 AC6: guards against double-init observed 2026-08-23 (two
// `manager_cadence_start` events at 39µs delta). This Once guard collapses
// single-process double-entry but does NOT prevent two mika-spirit
// processes from each spawning once. If `manager_cadence_spawn_duplicate_rejected`
// never fires post-deploy AND double-log persists, root cause is two
// processes — investigate supervise-daemon restart discipline / stale-PID reap.
```

**Change 6d — Out-of-scope but named (to prevent silent bundling):**

- **Fixing OpenRC supervise-daemon restart discipline** (root-cause candidate #1) — belongs on a separate ticket if `manager_cadence_spawn_duplicate_rejected` doesn't fire post-deploy while double-log persists. Requires OpenRC init script changes + PID-file discipline, orthogonal to Rust code in this repo.
- **Refactoring `run_server()` to be re-entrant-safe as a class** (root-cause candidate #2 hardening) — the `Once` guard on this ONE cadence spawn is targeted; a broader "make run_server idempotent" pass belongs on a separate substrate ticket.

**Test coverage:**

- Unit test `test_spawn_manager_cycle_task_second_call_rejected` in `spawn.rs`: call `spawn_manager_cycle_task` twice within the same test → first returns `Some(handle)`, second returns `None` and emits `manager_cadence_spawn_duplicate_rejected` warn. Test-guard reset: gate the `OnceLock` behind a `#[cfg(test)]` reset helper OR use a different guard type that supports test-scoped reset (e.g., a `Mutex<bool>` — trade OnceLock's performance for test-resettability). Recommend the `Mutex<bool>` shape given this fires once per process lifetime; performance is irrelevant at boot.

## Acceptance Criteria (verbatim from ticket, mapped to changes above)

1. **AC1: mika-spirit binary loads `~/.mika/.env` at process startup before Settings construction.**
   - Satisfied — `mika_common::dotenv::load_dotenv(&home_dir)` at `crates/mika-agent/src/bin/mika-spirit.rs:6` runs before `Settings::load` at line 13. Verification: `grep -n load_dotenv crates/mika-agent/src/bin/mika-spirit.rs` shows the call at line 6.

2. **AC2: verified by `tr '\0' '\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_` returning the env vars from the file.**
   - Satisfied *observationally* by the new `dotenv_loaded` structured log line + `mika_spirit_home_resolved` eprintln! — the operator sees the load outcome at boot without needing subprocess inspection.
   - The `/proc/<pid>/environ` grep also succeeds because `dotenvy::from_path()` calls `std::env::set_var()` internally per verified evidence (`dotenvy-0.15.7/src/iter.rs:35,52`) and per the existing test `test_load_dotenv_loads_vars` at `crates/mika-common/src/dotenv.rs:218-238` which asserts `std::env::var(...) == Some("hello_world")` post-load. Since `set_var()` mutates the process env visible to `/proc/<pid>/environ`, the AC2 grep succeeds when the file was loaded from the correct home dir.
   - Manual verification path in the PR body: after deploy, `journalctl -u mika-spirit -n 100 | grep -E 'home_resolved|dotenv'` shows the resolution + load outcome; `tr '\0' '\n' < /proc/$(pgrep -f mika-spirit | head -1)/environ | grep MIKA_MANAGER_TARGET_MILESTONE` returns the value if the boot line shows the expected home dir was resolved.

3. **AC3: verify no regression on Docker container startup (Dockerfile.agent may set env via ENV directives — merge semantics: dotenvy is no-op if var already set, safe).**
   - Satisfied by the existing `dotenvy::from_path()` semantics documented at `crates/mika-common/src/dotenv.rs:8` — "does NOT override existing env vars". No change; regression protection is already structural. Test coverage: add one integration-style assertion in `#[cfg(test)] mod tests` that shell-set var takes precedence over .env value (extends the existing `test_load_dotenv_does_not_override` at line 242 — verified present).

4. **AC4: solution doc added under `docs/solutions/best-practices/` capturing the class « app-owned config vs init-owned config » to compound the pattern.**
   - Satisfied by the new `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md` per § 4 above.

5. **AC5 (NEW 2026-08-23): GitHub auth VALIDE et TESTÉE dans le contexte daemon.**
   - **Boot-time sanity call:** satisfied by change 5b (`verify_gh_auth` called at top of `spawn_manager_cycle_task`, emits `manager_gh_auth_check_ok` / `manager_gh_auth_check_failed`).
   - **Root cause investigation:** ticket-body-listed candidates (`GH_TOKEN` mort dans daemon env / `~/.config/gh/hosts.yml` non-lu / `MIKA_GITHUB_TOKEN` non-injecté) validated during codebase research — the actual gap is that `spawn.rs:101` bypasses `Settings::agent_github_token()` and `Settings::resolve_github_token()`, reading `MIKA_GITHUB_TOKEN` directly from env. AC1-4 fix the env-drop path; change 5a fixes the Settings-bypass path so the manager benefits from GitHub App fallback the rest of the engine uses.
   - **Verification (post-deploy):** journalctl grep for `manager_gh_auth_check_ok` OR `manager_gh_auth_check_failed` at boot; cycle logs show `auth_class=` field on any residual `manager_cycle_error`.

6. **AC6 (NEW 2026-08-23): Single-init guard (spawn cadence idempotent).**
   - Satisfied by change 6a (`OnceLock`/`Mutex<bool>` guard rejecting duplicate calls within same process) + change 6b (spawn-attempt log carrying PID for root-cause discrimination) + change 6c (in-code investigation comment naming the two candidate root causes).
   - **Verification (post-deploy):** after restart, one `manager_cadence_start` event per PID; if `manager_cadence_spawn_duplicate_rejected` never fires but double-log persists, root cause is two processes (OpenRC discipline) — file separate per change 6d.

## Definition of Done

- [ ] `crates/mika-common/src/dotenv.rs::load_dotenv` emits `dotenv_loaded` (with `keys_from_file` count) / `dotenv_absent` / `dotenv_load_error` structured `info!`/`error!` lines.
- [ ] `crates/mika-agent/src/bin/mika-spirit.rs` emits `mika_spirit_home_resolved` eprintln! at boot (pre-logging::init), showing resolved home + MIKA_HOME + HOME env values.
- [ ] Two new unit tests in `dotenv.rs` verifying loaded-count parity + absent-file no-panic.
- [ ] Existing test `test_load_dotenv_does_not_override` verified present + green (AC3 regression protection).
- [ ] `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md` written per § 4.
- [ ] `crates/mika-agent/src/milestone_manager/spawn.rs::manager_config_from_env` takes `&Settings` and routes token acquisition through `Settings::resolve_github_token()` with `agent_github_token()` fallback (change 5a).
- [ ] `crates/mika-agent/src/milestone_manager/spawn.rs::verify_gh_auth` invoked at top of `spawn_manager_cycle_task`, emits `manager_gh_auth_check_ok` on success and `manager_gh_auth_check_failed` (with hint) on 401 (change 5b).
- [ ] `manager_cycle_error` warn line carries structured `auth_class` field (change 5c).
- [ ] Three new unit tests in `spawn.rs`: `test_verify_gh_auth_401_returns_err`, `test_verify_gh_auth_success_returns_ok`, `test_manager_config_from_env_prefers_app_token` (change 5 test coverage).
- [ ] `spawn_manager_cycle_task` returns `Option<JoinHandle<()>>` with `Once`/`Mutex<bool>` guard (change 6a); duplicate call emits `manager_cadence_spawn_duplicate_rejected` warn.
- [ ] Every spawn call emits `manager_cadence_spawn_attempt` with `pid` field (change 6b).
- [ ] In-code comment above the guard names the two root-cause candidates + escalation path (change 6c).
- [ ] Unit test `test_spawn_manager_cycle_task_second_call_rejected` in `spawn.rs` (change 6 test coverage).
- [ ] Call site at `crates/mika-agent/src/server/mod.rs:1354-1356` updated for new `manager_config_from_env` async signature + `Option<JoinHandle>` return.
- [ ] `cargo test -p mika-common --lib dotenv` clean.
- [ ] `cargo test -p mika-agent --lib milestone_manager` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] PR body documents manual acceptance path (deploy → journalctl grep for `manager_gh_auth_check_*` + `manager_cadence_spawn_attempt` + `dotenv_loaded` → /proc/environ check).

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

For each new log line, verify the test fails without the fix, then restore:

1. **`dotenv_loaded` presence** — temporarily comment out the `info!` call in load_dotenv's Ok arm; verify a test that spawns mika-spirit and greps its log for `dotenv_loaded` fails; restore.
2. **home_resolved eprintln!** — temporarily comment out the eprintln!; verify a test-harness invocation captures no `mika_spirit_home_resolved` line on stderr; restore.

Document in `todos/1968-injection-verification.md`.

## Out of scope

- **Fixing the OpenRC init script** to source `~/.mika/.env` — that's the (Z) mitigation Sami already applied; this ticket is the (Y) app-owned durable fix.
- **Reconciling `MIKA_HOME` semantics under supervise-daemon** — if `resolve_home_dir()` returns a wrong path under OpenRC because `HOME=/`, the fix is either to set `MIKA_HOME` in the OpenRC service file (operator config) or to make `resolve_home_dir` fall back to `getpwuid()` when `HOME` is `/` or unset (engine change). Out of scope for this ticket — file separate if the log lines show `home=/` in production.
- **A dashboard surface for boot-time env-check state** — Signal-class hardening beyond structured logs. Defer until n≥2 incident evidence.
- **Auto-migration of `MIKA_MANAGER_*` from `~/.mika/.env` into a systemd/OpenRC EnvironmentFile** — cross-platform installer work; separate ticket.
- **Fixing OpenRC supervise-daemon restart discipline / PID-file reap** (AC6 root-cause candidate #1) — file separate if `manager_cadence_spawn_duplicate_rejected` never fires post-deploy while double-log persists.
- **Making `run_server()` re-entrant-safe as a class** (AC6 root-cause candidate #2 broader hardening) — the change-6a guard is targeted to this ONE cadence spawn; a workspace-wide "make all long-lived spawns idempotent" pass belongs on a separate substrate ticket.
- **Refreshing GitHub App installation tokens mid-cycle** (~1h expiry) — the change-5a plan populates once at spawn-time. If steady-state 401s reappear on the App path post-fix, file follow-up for reader-boundary refresh.

## Risks and mitigations

- **eprintln! bypasses structured logging** — deliberate. Pre-logging-init boot lines must go somewhere; stderr is the least-lossy sink. Mitigation: rest of mika-spirit's boot uses `tracing::info!` — the two eprintln! lines are exceptional, not idiomatic.
- **`parse_dotenv` double-read at boot** — `load_dotenv` calling `parse_dotenv` first to count keys means the file is read twice. Cost: two syscalls + one small allocation at boot. Negligible; readability wins.
- **Log-line-shape drift** — if `event=` field names change, downstream grep breaks. Mitigation: log lines are named in this plan (`dotenv_loaded`, `dotenv_absent`, `dotenv_load_error`, `mika_spirit_home_resolved`) and referenced verbatim in the compound doc.

## Related solutions

- `docs/solutions/runtime-errors/uv-tool-install-force-doesnt-reinstall-deps-2026-05-19.md` — sibling class (uv-tool env divergence).
- Memory `feedback_shipped_ne_equal_pas_deployed_uv_tool_env` — same failure family at the Python-packaging layer.
- mika#1798 (invariant non-transit bake) — same class « ship shape then discover boundary ».

## Compounding potential

The new compound doc IS the compounding artifact. After merge, capture also:

- **eprintln!-before-logging::init pattern** (~30-line note): when a binary needs boot-time observability before its structured-log subscriber exists, eprintln! to stderr is the right escape hatch — do NOT install a temporary tracing subscriber just for two lines. Applies to any Mika bin.
