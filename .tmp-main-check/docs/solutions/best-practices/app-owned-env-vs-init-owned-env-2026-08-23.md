---
module: mika-common, mika-agent
tags: [config, env-vars, dotenv, boot-observability, supervise-daemon, systemd, docker]
problem_type: silent-config-drop
category: best-practices
---

# App-owned config vs init-owned config

## Class

Silent env-var drop across the process-launcher boundary.

## Failure family

Any binary spawned by a process supervisor that expects env vars set in a
project's `.env` file will silently see those vars as unset unless the
binary itself reads the file. The supervisor does not source arbitrary
files. This is not a bug in the supervisor — it is the *point* of the
supervisor: it starts your binary with a controlled, minimal environment.

Concrete launchers with this shape:

- **OpenRC `supervise-daemon`** (Gentoo, Alpine) launches the target with
  the environment it inherited from its own launch — typically empty
  except for `HOME`, `PATH`, and whatever `capset`/`respawn-delay` env the
  init script explicitly exported. There is no dotenv-load step.
- **systemd `Type=simple`** without `EnvironmentFile=` or explicit
  `Environment=` directives leaves the unit's env at the systemd default
  (essentially empty).
- **Docker** without `--env-file` or `ENV` directives in the Dockerfile
  gives you `$PATH`, `$HOME`, `$HOSTNAME`, and the vars you explicitly
  pass on `docker run`.

If the running binary reads its config from `dotenv`/`config-rs`/env vars,
and the launcher gave it none of the vars, **the binary comes up
mis-configured** — often silently, because the missing var maps to a
default (feature disabled, empty option, `None`).

## Rule

**The app owns its config.** The launcher does not.

`dotenvy::from_path("~/.mika/.env").ok()` at the top of `main()` (or an
equivalent) is the load-bearing gate. This is defense-in-depth against
every launcher shape: the binary loads its own env, so it doesn't matter
whether the launcher sourced the file, exported vars, or left the env
empty. The app comes up correctly regardless.

The alternative — teaching every launcher on every host to source
`~/.mika/.env` before spawning mika-spirit — is O(launchers × hosts) work
that has to stay in sync with the binary's env expectations. The
app-owned approach is O(1): one function call, one file, one contract.

## mika's implementation

`mika_common::dotenv::load_dotenv()` + `mika_common::home::resolve_home_dir()`
are called at every binary entry point: `mika-spirit`, `mika` CLI,
`calibrate`, `eval-diff`, `verify_bundled_skills`. Each binary owns the
same two-line pattern near the top of `main()`:

```rust
let home_dir = mika_common::home::resolve_home_dir()?;
mika_common::dotenv::load_dotenv(&home_dir);
```

`dotenvy::from_path()` (the underlying dotenv loader) explicitly does
NOT override vars already present in the process env, so a launcher-set
value (e.g. `MIKA_GITHUB_TOKEN` exported by an OpenRC init script for
per-agent identity) still wins over the file value. This is the merge
semantics you want: file provides defaults, launcher provides overrides.

## Diagnostic path

The ground-truth check for "did my binary actually load the expected
env?" is:

```bash
tr '\0' '\n' < /proc/$(pidof mika-spirit)/environ | grep MIKA_
```

If the expected var is absent, one of three subcauses applies:

1. **The binary does not call dotenv.** Check `main()`. Sibling binaries
   in the same crate can drift here — new binaries added after the
   pattern was established may forget the load call.
2. **The binary resolved the wrong home dir.** Under OpenRC `supervise-daemon`
   (and some systemd unit setups) the process may run with `HOME=/` or
   `HOME` unset. `dirs::home_dir()` (which `resolve_home_dir` delegates
   to when `MIKA_HOME` is unset) then returns `/` (or `None`), and the
   `.env` file at `/.mika/.env` doesn't exist → silent skip. Fix by
   setting `MIKA_HOME=/path/to/.mika` in the launcher's env, or by
   patching `resolve_home_dir` to fall back to `getpwuid()`.
3. **The `.env` file itself is missing/empty.** `dotenv_absent` in the
   log line means the file didn't exist; `dotenv_loaded keys_from_file=0`
   means it existed but was empty. Both are unambiguous now.

The mika#1968 additions turn this from "indirect subprocess inspection"
to "direct boot-line grep":

```
$ grep 'mika_spirit_home_resolved\|mika_spirit_env_check\|dotenv_' /var/log/mika/server.log
mika_spirit_home_resolved home=/root/.mika mika_home_env=<unset> home_env=/root
dotenv_loaded path=/root/.mika/.env keys_from_file=12
mika_spirit_env_check env_file_present=true env_file_keys=12 manager_target_set=true github_token_set=true
```

Three plain-text stderr lines that tell you: (a) which home dir the
binary picked, (b) whether the .env file was actually found and how many
keys it had (dotenv_loaded / dotenv_absent / dotenv_load_error — one of
these fires per boot), (c) whether the load-bearing vars are visible to
the process (env_file_present distinguishes "wrong home dir" from
"empty file at right dir"; manager_target_set + github_token_set
confirm the load-bearing MIKA_* vars reached the process env). If any
of those disagree with expectations, the failure class is immediately
obvious.

**Why plain-text stderr (not structured JSON):** these lines fire in
`main()` BEFORE `logging::init()` installs a JSON subscriber, so
`tracing::info!` calls would be silently dropped. `eprintln!` to stderr
is captured by OpenRC/systemd into the service log unconditionally.
Downstream aggregators that want structured events can also grep for
the `{"event":"dotenv_loaded",...}` JSON lines emitted post-init in
paths where a subscriber exists (mika CLI's per-invocation init;
`load_dotenv` calls made after `logging::init` on any future path);
both channels use the same event names + field names.

## When to use

Every long-running binary that expects to read `~/.mika/.env` (or any
project-level dotenv). The two-line pattern is cheap and pays for itself
the first time you diagnose a supervise-daemon-related env drop.

## When not to use

Short-lived CLI tools that run under the user's shell already inherit
the user's environment — a redundant dotenv load doesn't hurt but adds
nothing either. The rule targets **supervised** binaries, not
user-invoked ones.

## Precedent

- `feedback_shipped_ne_equal_pas_deployed_uv_tool_env` — same failure
  family at the Python-packaging layer. Package rebuild without env
  refresh leads to the same class of silent config drop; the fix shape
  is the same (the app owns its config resolution, not the packaging
  tool).
- mika#1798 — invariant non-transit doctrine bake. Same class in a
  different domain: "ship shape then discover boundary" only works when
  the shipped binary owns the invariants it depends on, not when it
  assumes an upstream layer will provide them.

## References

- Founding incident: mika#1968 (2026-08-22 mika-manager cadence
  observability blackout — 24h of zero heartbeats because
  `MIKA_MANAGER_TARGET_MILESTONE` was in `~/.mika/.env` but the
  supervise-daemon-launched mika-spirit process never saw it).
- Implementation: `crates/mika-common/src/dotenv.rs::load_dotenv`,
  `crates/mika-agent/src/bin/mika-spirit.rs::main`.
- Verification pattern: `tr '\0' '\n' < /proc/<pid>/environ | grep <VAR>`
  is Linux-only. On macOS use `ps eww <pid>`.

---

## Sibling pattern 1 — Dual-channel observability for pre-init boot events

**Class:** Silent event drop across the boot-time subscriber-install
boundary.

**Failure family:** Any binary that emits `tracing::info!/error!` events
BEFORE `logging::init()` installs a subscriber will have those events
silently dropped. This most commonly happens when logging config depends
on settings that themselves depend on env vars loaded from the filesystem
— the ordering `load_dotenv → Settings::load → logging::init` is correct
for correctness reasons (settings needs env, log-format needs settings)
but wrong for observability if load-side events use `tracing!`.

**Rule:** boot-time state that fires before the subscriber must go to a
subscriber-independent sink. In Rust that's `eprintln!` (or `println!`);
stderr is captured by OpenRC / systemd / Docker log drivers
unconditionally. Structured `tracing!` events remain valuable for
post-init paths (aggregation, JSON logs), so the shape that survives
review is **dual-channel emission**: emit both, with the same event names
and field names, so a single operator grep hits either sink.

**mika's implementation:** `mika_common::dotenv::load_dotenv` emits the
same three states (`dotenv_loaded` / `dotenv_absent` / `dotenv_load_error`)
via both `eprintln!` (durable pre-subscriber) and `info!/error!`
(structured post-subscriber). Same event names + field names on both
channels. `mika-spirit::main` also emits `mika_spirit_home_resolved`
and `mika_spirit_env_check` via `eprintln!` for the same reason (they
fire before the subscriber exists, and no post-init duplicate is
warranted for those single-emission boot lines).

**When to use:** every long-running service binary that has state to
observe before `logging::init()`. If your service currently uses only
`tracing!` in `main()` before subscriber install, its pre-init events
are silently lost.

**When not to use:** short-lived CLIs that install a subscriber
per-invocation don't need the `eprintln!` channel — the subscriber is
up before any interesting state fires. Adding `eprintln!` there would
just be noise on stderr.

---

## Sibling pattern 2 — Verifier fidelity: fail-loud on parse errors

**Class:** Silent-pass by inversion — a boot-time verifier whose job is
to catch a bad state falls back to a "default" success value on parse
failure, effectively hiding the exact class of failure it exists to
detect.

**Founding incident:** mika#1968 verify_gh_auth initially did
`serde_json::from_str(...).and_then(...).unwrap_or(0)` on the GitHub
`/rate_limit` response. Empty body / malformed JSON / schema drift all
returned `Ok(0)` — which then logged as `manager_gh_auth_check_ok
rate_limit_remaining=0`. On a healthy authenticated account,
`remaining=0` is impossible (5000/hr baseline). An operator seeing
`check_ok` would assume auth is live when it may be silently degraded —
the exact silent-pass class the verifier was written to prevent.

**Rule:** for a boot-time verifier, treat any deviation from the
expected shape as failure, not as a defaulted success. `unwrap_or(sentinel)`
in a verifier IS the class the verifier exists to catch. The verifier
should return `Err(discriminator)` with a distinct classification (e.g.,
`parse_failure:` prefix on the error snippet) so operators can grep for
the schema-drift class without confusing it with genuine 401/403/network
errors.

**Analog in test discipline:** the same shape appears in tests as
"assertion elided into the expected path" — a test that swallows a
`.unwrap_err()` into the success arm hides the very regression it was
written to catch. Never fall-back to success in a verifier or a
regression test.

**When to use:** every boot-time sanity check, every pre-flight gate,
every "can this false-pass?" surface (CI/CD gates, merge-blockers,
build/deploy verification steps, health probes). If the mechanism's job
is to catch a class of bad state, its parse-error path must be fail-loud.

**When not to use:** best-effort optional data fetch where partial data
is genuinely useful (e.g., an analytics counter that's OK to be
approximate). But that's not a verifier — that's a data collection path.
The distinction matters.

**Cross-reference:** this pattern generalizes the pre-existing memory
`feedback_prompt_enforcement_fragile` from the code-review discipline
domain to the boot-time gate discipline.
