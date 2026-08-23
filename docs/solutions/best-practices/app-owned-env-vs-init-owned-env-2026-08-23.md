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
