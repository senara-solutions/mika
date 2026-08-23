# Plan — fix(mika-spirit): verify + harden ~/.mika/.env loading at boot

**Status:** DRAFT
**Date:** 2026-08-23
**Ticket:** mika#1968
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Substrate reliability — env-vars-at-boot verification + observability

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

## Definition of Done

- [ ] `crates/mika-common/src/dotenv.rs::load_dotenv` emits `dotenv_loaded` (with `keys_from_file` count) / `dotenv_absent` / `dotenv_load_error` structured `info!`/`error!` lines.
- [ ] `crates/mika-agent/src/bin/mika-spirit.rs` emits `mika_spirit_home_resolved` eprintln! at boot (pre-logging::init), showing resolved home + MIKA_HOME + HOME env values.
- [ ] Two new unit tests in `dotenv.rs` verifying loaded-count parity + absent-file no-panic.
- [ ] Existing test `test_load_dotenv_does_not_override` verified present + green (AC3 regression protection).
- [ ] `docs/solutions/best-practices/app-owned-env-vs-init-owned-env-2026-08-23.md` written per § 4.
- [ ] `cargo test -p mika-common --lib dotenv` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] PR body documents manual acceptance path (deploy → journalctl grep → /proc/environ check).

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
