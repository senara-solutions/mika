# mika-permission-policy

Mika-side per-binary safety functions for the [claude-pilot](https://github.com/senara-solutions/claude-pilot) per-spawn permission-policy evaluator. Ships the private allow/deny CONTENTS that live on Mika's side of the SSC OSS boundary; claude-pilot ships the generic engine (`per_spawn.py`, empty `DEFAULT_POLICY`) and loads this plugin at runtime.

Design source: [mika#1708](https://github.com/senara-solutions/mika/issues/1708) architect-ratified spec, [mika#1817](https://github.com/senara-solutions/mika/issues/1817) this plugin.

## Install

The plugin must be importable by claude-pilot's Python environment. If claude-pilot was installed via `uv tool install`, add this plugin to the same tool env:

```bash
# From mika/ root (this repo)
uv tool install --reinstall --force --editable ../claude-pilot \
  --with-editable ./tools/mika_permission_policy
```

Or the Makefile target:

```bash
make install-permission-policy-plugin
```

Once installed, flip the two env vars claude-pilot reads at handler creation:

```bash
export MIKA_PERMISSION_POLICY_MODE=per_spawn
export MIKA_PERMISSION_POLICY_MODULE=mika_permission_policy:get_policy
```

Set them per-session (`env VAR=... mika ask ...`) for canary testing, or persistently in `~/.mika/.env`.

## What ships

`get_policy()` returns a `dict[str, PolicyFn]` where each entry maps a binary basename to a safety function of shape `(argv: list[str], cwd: str) -> bool`. The registry mirrors the auto-approve semantics of `claude_pilot.tier1` — parity is the correctness contract for Phase 1 opt-in (see `tests/test_binaries.py`).

Initial binary set (mika#1708 AC3):

| Binary | Guard |
|---|---|
| `grep` / `egrep` / `fgrep` | allow-all read-only (accepted risk: GNU-grep assumption, see below) |
| `awk` | deny `system()`, `getline \| "cmd"`, `print \| "cmd"` |
| `sed` | deny `-i` / `--in-place` in any short-flag cluster |
| `cat` / `ls` / `head` / `tail` / `wc` / `stat` / `file` / `which` / `type` / `pwd` / `date` / `uniq` / `tr` / `cut` / `diff` / `comm` / `realpath` / `readlink` / `dirname` / `basename` / `test` / `[` | allow-all read-only |
| `find` | deny `-delete`, `-fprintf`/`-fprint`/`-fprint0`/`-fls`, `-exec/-execdir/-ok/-okdir <cmd>` unless `<cmd>` is in the FIND_EXEC_SAFE_COMMANDS allowlist |
| `sort` | deny `-o` / `--output` (all abbreviations and cluster positions) |
| `git` | subcommand allowlist + deny `--force`, `push origin main/master`, `branch -D`, `reset` |
| `gh` | domain+verb allowlist + deny `api` mutation flags (`-X`, `--method`, `-f`, `-F`, `--field`, `--raw-field`, `--input`) |
| `cargo` | subcommand allowlist (deny `publish`, `install`, `run`) |
| `make` | closed-world target allowlist (`verify-bundled-skills` only, no extra tokens) |
| `sqlite3` | deny `DROP`/`DELETE`/`INSERT`/`UPDATE`/`ALTER`/`CREATE`/`TRUNCATE`/`VACUUM` (case-insensitive) |
| `bash` / `sh` | deny `-c` (any short-flag cluster containing 'c') |
| `echo` / `printf` / `true` / `false` / `:` | allow-all no-op |

Binaries not in the registry deny by default — `per_spawn.evaluate()` treats a missing entry as a reject, dropping through to classic tier2 / relay during Phase 1 opt-in.

## Adding a binary

1. Add the safety function to `mika_permission_policy/_binaries.py` with signature `(argv: list[str], cwd: str) -> bool`.
2. Register it in `mika_permission_policy/__init__.py:get_policy()`.
3. Add tests to `tests/test_binaries.py` covering known-safe and known-deny argv shapes.
4. Update the table above.

Parity discipline: if `tier1.py` allows a shape and this plugin denies it, that is a REGRESSION unless the deny is on a shape the security review of tier1 already flagged. Prefer expanding tier1's own accepted-risks documentation over silently diverging here.

## Accepted risks (inherited from tier1)

- **GNU grep assumption.** `grep`/`egrep`/`fgrep` treat the invocation as read-only. GNU grep is; ugrep (a drop-in `grep` on some Gentoo/BSD/Homebrew hosts) has `--filter`/`--pager`/`--view` that execute commands. tier1 documented this as an accepted risk on the pilot's standard-Linux target where GNU grep 3.12 is resolved. If the deployment target changes, drop grep from the registry entirely — do NOT try to parse `--filter` (inner-arg lexing is a forbidden posture per `claude-pilot/docs/solutions/security-issues/command-string-policy-allow-rules-are-compound-unsafe.md § 4`).
- **awk `s/foo/bar/e` and sed `e` command inside script strings.** Detecting these requires parsing the awk/sed script — out of scope. tier1 does not detect them either (raw-string regex has the same blind spot). Same accepted risk, same downstream mitigation (relay-level judgment on unusual inputs).
- **Missing entries auto-deny.** Any binary not in the registry rejects. If Mika's operator idioms include a binary not listed here, the dispatch falls through to classic tier2 / relay during Phase 1 opt-in, or to a hard-deny once Phase 3 retires classic. Monitor `perm_policy_rollback` audit events during Phase 2 and expand this registry as evidence arrives.

## What this plugin does NOT check

per_spawn's engine already refuses these at the raw-source level, so per-binary functions never see them:

- Command substitution (`` `...` ``, `$(...)`), heredocs (`<<`), process substitution (`<(...)`, `>(...)`), arithmetic expansion (`$((...))`)
- Control flow (`if`, `for`, `while`, `case`, `select`, `until`, functions)
- Dynamic execution builtins (`eval`, `source`, `.`)
- Chain safety across multiple spawns — each spawn is evaluated independently; every one must pass

Path containment for filesystem targets is out of scope (that is the Write/Edit tool's concern, not Bash's — mirrors tier1).

## Structural typing note

We deliberately do NOT import `claude_pilot.per_spawn.PolicyFn` at package-import time. The protocol is `Callable[[list[str], str], bool]` — plain structural typing — so this plugin can be installed alongside any cpp version implementing the same protocol without a version pin coupling. This keeps SSC OSS boundary discipline (per mika#1708 landing shape): the plugin is a downstream consumer of the protocol, not a dependent of the exact cpp release.
