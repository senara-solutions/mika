---
module: build
tags: [version, git-hash, build-stamp, deploy-verification, provenance, build-rs, option_env, clap, mika-cli, mika-gateway, mika-spirit]
problem_type: missing-capability
category: infrastructure
---

# A binary must be able to state its own commit without a configured server

## Problem (mika#2066)

No installed mika binary could say which commit it was built from without a
configured, running server. `mika#354` already captured the short git hash at
compile time (`mika-gateway/build.rs` → `GIT_HASH`) and served it on
`GET /version`, but that reading required an HTTP request against a server that
had already loaded its config. Worse, the `--version` flag of the same binary
loaded configuration *before* answering:

```
$ mika-gateway --version
Error: Failed to load gateway settings: missing configuration field "database_url".
```

And `mika-cli` had no `build.rs` at all — the `mika` binary the operator
actually holds never carried the hash. So a deploy could only be verified by
**provenance** (clean tree at commit X, `make install` exit 0, binary mtime
after merge) — three correct facts, none of which interrogate the binary
itself. The gap is paid exactly when it costs most: confirming that the fix you
shipped is the one now running. On `control-monitor` the same day, root proved
its deploy in one command: `cm --version` → the commit stamp.

## Solution — one shared capture, read by every binary, before any config

Three moves, mapped to the acceptance criteria:

1. **Single shared capture (AC3).** `crates/mika-common/build.rs` runs
   `git rev-parse --short=8 HEAD` and emits `cargo:rustc-env=GIT_HASH=…`.
   `crates/mika-common/src/build_info.rs` exposes it:

   ```rust
   pub const GIT_HASH: &str = match option_env!("GIT_HASH") {
       Some(h) => h,
       None => "unknown",
   };
   pub fn version_string() -> String { format!("{VERSION} ({GIT_HASH})") }
   ```

   The key fact: **`option_env!` resolves in the crate whose build script set
   the variable**. Capturing in `mika-common` — the crate every binary links —
   lets `mika`, `mika-gateway`, and `mika-spirit` all read one stamp instead of
   recopying a `build.rs` per crate. The old `mika-gateway/build.rs` capture and
   its local `option_env!` in `routes.rs` are removed; `GET /version` now reads
   `mika_common::build_info::GIT_HASH`, behavior unchanged.

2. **`mika --version` (AC1)** wires clap's `version` to the shared stamp:
   `#[command(version = mika_common::build_info::version_static())]`. clap prints
   `mika 0.12.2 (<commit>)` and short-circuits before the command dispatch that
   would load config.

3. **`mika-gateway` / `mika-spirit` `--version` (AC2)** are not clap-driven at
   the top level, so they short-circuit manually. The **first** statement of
   each `main`, before rustls / dotenv / settings / home resolution:

   ```rust
   mika_common::build_info::print_version_if_requested("mika-gateway");
   ```

   which scans argv for `--version`/`-V`, prints `{bin} {version_string}`, and
   exits 0. An unconfigured binary can now state what it is.

Out-of-git builds (Docker layer, source tarball) yield `GIT_HASH = "unknown"` —
never a failed compile.

## Why `option_env!` is the load-bearing constraint

The instinct is to capture the hash in each binary's own `build.rs`. That works
but duplicates the capture (the thing AC3 forbids) and drifts. You cannot
centralize by putting the capture in a shared `build.rs` and reading it from
another crate's source: `cargo:rustc-env` only reaches the crate being built.
The capture and every `option_env!("GIT_HASH")` read must live in the **same
crate**. `mika-common` is that crate because everything links it.

## Testing

- `mika-common::build_info` unit tests: the stamp is never empty (real commit or
  the explicit `unknown` fallback); `version_string` pairs semver with the stamp.
- One integration test per binary (`tests/version_flag.rs` in mika-cli,
  mika-gateway, mika-agent) runs the built binary with `--version`, **with every
  `MIKA_*` var removed from the environment**, and asserts exit 0 plus a
  non-empty parenthesized stamp. For gateway and spirit, a clean exit *is* the
  proof that config is not required (AC2).

## Gotcha — `rerun-if-changed` under a subdirectory build script

The old `mika-gateway/build.rs` emitted `rerun-if-changed=.git/HEAD`, but a
build script's CWD is its crate's manifest dir, so that path pointed at
`crates/mika-gateway/.git/HEAD` — which never exists. The stamp silently went
stale until an unrelated recompile. The mika-common build script resolves the
real git dir with `git rev-parse --absolute-git-dir` and watches `HEAD` + `refs`
there.

## Related

- `mika#354` — original `/version` endpoint and gateway hash capture (kept; this
  ticket only re-sources it).
- `feedback_binary_staleness_vs_main` exists because comparing a binary mtime to
  a merge date was the best available approximation. With the binary now
  interrogable, that approximation is a candidate for retirement.
